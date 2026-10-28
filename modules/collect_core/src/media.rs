//! The photo pipeline (SPEC section 5).
//!
//! `CLAUDE.md`: read location and time, then remove all metadata before
//! upload, and never upload the original file. Everything stored is public and
//! nothing on Storage can be deleted, so a GPS tag that escapes once is public
//! forever — which is why `process` re-reads its own output and fails rather
//! than return a file it cannot show is clean.
//!
//! Order matters: EXIF is read from the original, because cleaning destroys it.

use crate::canonical;
use exif::{In, Tag, Value as ExifValue};
use sha2::{Digest, Sha256};
use std::io::Cursor;

/// Container formats a submission may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Webp,
    /// Recognised so it can be refused with a reason rather than as "not an
    /// image": the contract allows `image/heic`, but decoding it needs a
    /// system codec we cannot assume.
    Heic,
}

impl Format {
    pub fn mime(self) -> &'static str {
        match self {
            Format::Jpeg => "image/jpeg",
            Format::Png => "image/png",
            Format::Webp => "image/webp",
            Format::Heic => "image/heic",
        }
    }

    /// Identify a format from its leading bytes. The extension is not
    /// consulted: it is the submitter's claim, not the file's content.
    pub fn sniff(bytes: &[u8]) -> Option<Format> {
        if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Some(Format::Jpeg);
        }
        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Some(Format::Png);
        }
        if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
            return Some(Format::Webp);
        }
        if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
            let brand = &bytes[8..12];
            if matches!(
                brand,
                b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"heim" | b"heis"
            ) {
                return Some(Format::Heic);
            }
        }
        None
    }
}

/// EXIF orientation, 1 to 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Orientation(pub u16);

impl Orientation {
    pub const NORMAL: Orientation = Orientation(1);

    /// Does applying this orientation change the pixels?
    pub fn is_identity(self) -> bool {
        self.0 == 1 || !(1..=8).contains(&self.0)
    }

    /// Does applying it swap width and height?
    pub fn transposes(self) -> bool {
        matches!(self.0, 5..=8)
    }
}

/// A location read out of EXIF GPS tags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExifLocation {
    pub lat: f64,
    pub lon: f64,
    /// From `GPSHorizontalPositioningError`, when the camera recorded it.
    pub accuracy_m: Option<f64>,
}

/// What section 5 step 1 reads, used to prefill `location` and `datetime`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoExif {
    pub location: Option<ExifLocation>,
    /// `DateTimeOriginal` combined with `OffsetTimeOriginal`, as RFC 3339.
    ///
    /// Only set when the photo carried an offset. A `datetime` field needs an
    /// offset to name an instant, and there is no correct offset to invent for
    /// a photo that did not record one.
    pub datetime: Option<String>,
    /// The raw `DateTimeOriginal` reading with no offset applied, for a UI
    /// that wants to show it and ask.
    pub datetime_naive: Option<String>,
    pub orientation: Orientation,
}

impl Default for Orientation {
    fn default() -> Self {
        Orientation::NORMAL
    }
}

/// A cleaned file, ready to upload.
#[derive(Debug, Clone)]
pub struct Cleaned {
    /// The bytes to upload. Never the original.
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    /// SHA-256 of `bytes`, matching the `sha256` member of a media entry.
    pub sha256: String,
    /// What was read from the original before cleaning.
    pub exif: PhotoExif,
    /// Whether the pixels were re-encoded, or only container metadata removed.
    pub reencoded: bool,
}

impl Cleaned {
    /// The media entry for this file, once Storage has given it a CID.
    pub fn media_entry(&self, cid: &str) -> serde_json::Value {
        serde_json::json!({
            "cid": cid,
            "sha256": self.sha256,
            "mime": self.mime,
            "bytes": self.bytes.len(),
            "width": self.width,
            "height": self.height,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a JPEG, PNG or WebP image")]
    UnknownFormat,
    #[error("HEIC decoding is not available in this build")]
    HeicUnsupported,
    #[error("file is {bytes} bytes, over the {limit_mb} MB limit")]
    TooLarge { bytes: usize, limit_mb: f64 },
    #[error("image could not be decoded: {0}")]
    Decode(String),
    #[error("cleaned image could not be encoded: {0}")]
    Encode(String),
    /// The self-check after cleaning. Returning the file anyway would publish
    /// whatever was left behind.
    #[error("cleaned file still carries metadata ({0}); refusing to upload it")]
    NotClean(String),
}

/// Quality for a JPEG that has to be re-encoded.
///
/// Only reached when the orientation has to be baked into the pixels; an
/// already-upright JPEG keeps its original pixels untouched.
const JPEG_QUALITY: u8 = 92;

/// Read the EXIF of an original file. Never fails: a photo with no EXIF, or
/// with EXIF we cannot parse, simply prefills nothing.
pub fn read_exif(bytes: &[u8]) -> PhotoExif {
    let mut out = PhotoExif::default();
    let Ok(reader) = exif::Reader::new().read_from_container(&mut Cursor::new(bytes)) else {
        return out;
    };

    if let Some(field) = reader.get_field(Tag::Orientation, In::PRIMARY)
        && let Some(v) = field.value.get_uint(0)
    {
        out.orientation = Orientation(v as u16);
    }

    out.location = read_gps(&reader);

    let naive = reader
        .get_field(Tag::DateTimeOriginal, In::PRIMARY)
        .and_then(|f| ascii_of(&f.value));
    let offset = reader
        .get_field(Tag::OffsetTimeOriginal, In::PRIMARY)
        .and_then(|f| ascii_of(&f.value));

    if let Some(naive) = naive {
        // EXIF writes "YYYY:MM:DD HH:MM:SS"; RFC 3339 wants
        // "YYYY-MM-DDTHH:MM:SS" plus an offset.
        let rfc = naive
            .get(..10)
            .map(|d| d.replace(':', "-"))
            .and_then(|d| naive.get(11..19).map(|t| format!("{d}T{t}")));
        out.datetime_naive = rfc.clone();
        if let (Some(stamp), Some(offset)) = (rfc, offset) {
            let candidate = format!("{stamp}{}", offset.trim());
            // Only hand back something a `datetime` field will accept.
            if chrono::DateTime::parse_from_rfc3339(&candidate).is_ok() {
                out.datetime = Some(candidate);
            }
        }
    }

    out
}

fn read_gps(reader: &exif::Exif) -> Option<ExifLocation> {
    let lat = degrees(reader, Tag::GPSLatitude)?;
    let lon = degrees(reader, Tag::GPSLongitude)?;
    let lat_ref = reader
        .get_field(Tag::GPSLatitudeRef, In::PRIMARY)
        .and_then(|f| ascii_of(&f.value))
        .unwrap_or_else(|| "N".into());
    let lon_ref = reader
        .get_field(Tag::GPSLongitudeRef, In::PRIMARY)
        .and_then(|f| ascii_of(&f.value))
        .unwrap_or_else(|| "E".into());

    let lat = if lat_ref.starts_with('S') { -lat } else { lat };
    let lon = if lon_ref.starts_with('W') { -lon } else { lon };
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }

    let accuracy_m = reader
        .get_field(Tag::GPSHPositioningError, In::PRIMARY)
        .and_then(|f| rationals(&f.value).first().copied())
        .filter(|v| v.is_finite() && *v >= 0.0);

    Some(ExifLocation {
        lat,
        lon,
        accuracy_m,
    })
}

/// Degrees, minutes and seconds as a single decimal degree value.
fn degrees(reader: &exif::Exif, tag: Tag) -> Option<f64> {
    let parts = rationals(&reader.get_field(tag, In::PRIMARY)?.value);
    if parts.len() < 3 {
        return None;
    }
    let value = parts[0] + parts[1] / 60.0 + parts[2] / 3600.0;
    value.is_finite().then_some(value)
}

fn rationals(value: &ExifValue) -> Vec<f64> {
    match value {
        ExifValue::Rational(list) => list.iter().map(|r| r.to_f64()).collect(),
        ExifValue::SRational(list) => list.iter().map(|r| r.to_f64()).collect(),
        _ => Vec::new(),
    }
}

fn ascii_of(value: &ExifValue) -> Option<String> {
    match value {
        ExifValue::Ascii(parts) => parts.first().map(|b| {
            String::from_utf8_lossy(b)
                .trim_end_matches('\0')
                .to_string()
        }),
        _ => None,
    }
}

/// Read the metadata, then produce a cleaned file to upload.
///
/// `max_mb_each` is checked against the file as submitted, which is what
/// section 5 step 4 bounds.
pub fn process(original: &[u8], max_mb_each: f64) -> Result<Cleaned, Error> {
    let limit = (max_mb_each * 1024.0 * 1024.0) as usize;
    if original.len() > limit {
        return Err(Error::TooLarge {
            bytes: original.len(),
            limit_mb: max_mb_each,
        });
    }

    let format = Format::sniff(original).ok_or(Error::UnknownFormat)?;
    if format == Format::Heic {
        return Err(Error::HeicUnsupported);
    }

    let exif = read_exif(original);

    // Re-encoding is lossy for JPEG, so avoid it when the only thing to do is
    // drop metadata. PNG is lossless either way.
    let lossless_strip_possible = exif.orientation.is_identity()
        && matches!(format, Format::Jpeg | Format::Webp | Format::Png);

    let (bytes, width, height, reencoded) = if lossless_strip_possible {
        match strip_in_place(original, format) {
            Some(stripped) => {
                let (w, h) = dimensions(&stripped)?;
                (stripped, w, h, false)
            }
            // Container we could not rewrite safely — fall back to a full
            // re-encode rather than upload something still carrying tags.
            None => reencode(original, exif.orientation)?,
        }
    } else {
        reencode(original, exif.orientation)?
    };

    let cleaned = Cleaned {
        sha256: hex::encode(Sha256::digest(&bytes)),
        bytes,
        mime: format.mime(),
        width,
        height,
        exif,
        reencoded,
    };

    // The self-check. A strip that silently missed a segment, or an encoder
    // that copied metadata forward, would otherwise publish it.
    if let Some(found) = remaining_metadata(&cleaned.bytes) {
        return Err(Error::NotClean(found));
    }

    Ok(cleaned)
}

/// What metadata, if any, a finished file still carries.
///
/// `None` means clean. Used by `process` on its own output and available to
/// callers that want to check a file they did not produce.
pub fn remaining_metadata(bytes: &[u8]) -> Option<String> {
    let found = read_exif(bytes);
    if found.location.is_some() {
        return Some("GPS tags".into());
    }
    if found.datetime_naive.is_some() {
        return Some("DateTimeOriginal".into());
    }
    // Any readable EXIF block at all is a failure, even with no field we
    // happen to look at.
    if exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .is_ok()
    {
        return Some("an EXIF block".into());
    }
    if let Some(Format::Jpeg) = Format::sniff(bytes)
        && jpeg_segments(bytes).is_some_and(|segments| {
            segments
                .iter()
                .any(|(marker, _)| is_metadata_segment(*marker))
        })
    {
        return Some("a JPEG metadata segment".into());
    }
    if let Some(Format::Webp) = Format::sniff(bytes)
        && webp_chunks(bytes)
            .is_some_and(|chunks| chunks.iter().any(|(id, _)| matches!(id, b"EXIF" | b"XMP ")))
    {
        return Some("a WebP EXIF or XMP chunk".into());
    }
    None
}

fn dimensions(bytes: &[u8]) -> Result<(u32, u32), Error> {
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| Error::Decode(e.to_string()))?
        .into_dimensions()
        .map_err(|e| Error::Decode(e.to_string()))
}

/// Decode, apply the orientation to the pixels, and encode afresh.
///
/// Re-encoding drops every metadata block as a side effect, which is the
/// point; `strip_in_place` exists so this only runs when it must.
fn reencode(original: &[u8], orientation: Orientation) -> Result<(Vec<u8>, u32, u32, bool), Error> {
    let reader = image::ImageReader::new(Cursor::new(original))
        .with_guessed_format()
        .map_err(|e| Error::Decode(e.to_string()))?;
    let format = reader.format();
    let image = reader.decode().map_err(|e| Error::Decode(e.to_string()))?;
    let image = apply_orientation(image, orientation);

    let (width, height) = (image.width(), image.height());
    let mut out = Cursor::new(Vec::new());
    match format {
        Some(image::ImageFormat::Jpeg) => {
            // Chroma subsampling and quality are the encoder's defaults
            // otherwise; 92 keeps a visible-detail margin for a photo that is
            // being re-encoded only to bake in its rotation.
            let mut encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
            encoder
                .encode_image(&image)
                .map_err(|e| Error::Encode(e.to_string()))?;
        }
        Some(image::ImageFormat::Png) => {
            image
                .write_to(&mut out, image::ImageFormat::Png)
                .map_err(|e| Error::Encode(e.to_string()))?;
        }
        Some(image::ImageFormat::WebP) => {
            // This encoder writes lossless WebP, so a lossy source can grow.
            // Reached only when the orientation has to be baked in.
            image
                .write_to(&mut out, image::ImageFormat::WebP)
                .map_err(|e| Error::Encode(e.to_string()))?;
        }
        _ => return Err(Error::UnknownFormat),
    }
    Ok((out.into_inner(), width, height, true))
}

/// The eight EXIF orientations as pixel transforms.
fn apply_orientation(image: image::DynamicImage, orientation: Orientation) -> image::DynamicImage {
    match orientation.0 {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate270().fliph(),
        8 => image.rotate270(),
        // 1, and anything out of range, is already upright.
        _ => image,
    }
}

/// Remove metadata from the container without touching the pixels.
///
/// `None` when the container could not be parsed, which sends the caller to a
/// full re-encode rather than leaving metadata in place.
fn strip_in_place(bytes: &[u8], format: Format) -> Option<Vec<u8>> {
    match format {
        Format::Jpeg => strip_jpeg(bytes),
        Format::Webp => strip_webp(bytes),
        // PNG metadata lives in ancillary chunks; re-encoding is lossless, so
        // there is nothing to gain from rewriting the container by hand.
        Format::Png => None,
        Format::Heic => None,
    }
}

/// Which JPEG markers carry metadata we must drop.
///
/// APP0 (JFIF) holds density only, and APP2 is the ICC colour profile —
/// dropping that would change how the photo looks, and neither is personal
/// data. Everything else in the APPn range can and does carry EXIF, XMP, IPTC
/// or maker notes, so it goes. COM goes too: a free-text comment.
fn is_metadata_segment(marker: u8) -> bool {
    matches!(marker, 0xE1 | 0xE3..=0xEF | 0xFE)
}

/// Split a JPEG into its segments as (marker, payload-with-length) pairs.
///
/// Stops at SOS, after which the entropy-coded data runs to the end.
fn jpeg_segments(bytes: &[u8]) -> Option<Vec<(u8, std::ops::Range<usize>)>> {
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut segments = Vec::new();
    let mut at = 2;
    loop {
        if at + 1 >= bytes.len() {
            return None;
        }
        if bytes[at] != 0xFF {
            return None;
        }
        let marker = bytes[at + 1];
        // Start of scan, or end of image: no more headers to read.
        if marker == 0xDA || marker == 0xD9 {
            segments.push((marker, at..bytes.len()));
            return Some(segments);
        }
        // Standalone markers carry no length.
        if (0xD0..=0xD8).contains(&marker) || marker == 0x01 {
            segments.push((marker, at..at + 2));
            at += 2;
            continue;
        }
        if at + 3 >= bytes.len() {
            return None;
        }
        let length = u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]) as usize;
        if length < 2 || at + 2 + length > bytes.len() {
            return None;
        }
        segments.push((marker, at..at + 2 + length));
        at += 2 + length;
    }
}

fn strip_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    let segments = jpeg_segments(bytes)?;
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&[0xFF, 0xD8]);
    for (marker, range) in segments {
        if is_metadata_segment(marker) {
            continue;
        }
        out.extend_from_slice(&bytes[range]);
    }
    Some(out)
}

/// Split a WebP file into its RIFF chunks as (id, payload range) pairs.
fn webp_chunks(bytes: &[u8]) -> Option<Vec<([u8; 4], std::ops::Range<usize>)>> {
    if bytes.len() < 12 || !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let mut chunks = Vec::new();
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id: [u8; 4] = bytes[at..at + 4].try_into().ok()?;
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let end = at + 8 + size;
        if end > bytes.len() {
            return None;
        }
        chunks.push((id, at..end));
        // RIFF chunks are padded to an even length.
        at = end + (size & 1);
    }
    Some(chunks)
}

fn strip_webp(bytes: &[u8]) -> Option<Vec<u8>> {
    let chunks = webp_chunks(bytes)?;
    let mut body = Vec::with_capacity(bytes.len());
    body.extend_from_slice(b"WEBP");
    for (id, range) in chunks {
        if matches!(&id, b"EXIF" | b"XMP ") {
            continue;
        }
        let chunk = &bytes[range];
        body.extend_from_slice(chunk);
        if chunk.len() % 2 == 1 {
            body.push(0);
        }
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Some(out)
}

/// Prefill values a form can take from a photo, for the fields a protocol
/// says may be filled from one.
///
/// Returns the `location` and `datetime` values as they would be stored. The
/// location arrives as method `photo`; a `rounded_1km` field still has to go
/// through `fields::apply_store_rule` before the value is kept.
pub fn prefills(exif: &PhotoExif) -> (Option<serde_json::Value>, Option<String>) {
    let location = exif.location.map(|l| {
        let mut value = serde_json::json!({
            "lat": number(l.lat),
            "lon": number(l.lon),
            "method": "photo",
            "rounded": false,
        });
        if let Some(accuracy) = l.accuracy_m {
            value["accuracy_m"] = number(accuracy);
        }
        value
    });
    (location, exif.datetime.clone())
}

fn number(v: f64) -> serde_json::Value {
    serde_json::Number::from_f64(v)
        .map(serde_json::Value::Number)
        .unwrap_or(serde_json::Value::Null)
}

/// Canonical JSON of a media entry, for callers that need the signing bytes.
pub fn media_entry_bytes(entry: &serde_json::Value) -> Result<Vec<u8>, canonical::Error> {
    canonical::to_bytes(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};

    // ---- building a JPEG that actually carries GPS EXIF ----
    //
    // The privacy test is only worth anything against a file that really has
    // the tags, so the fixture is assembled by hand rather than asserted
    // about in the abstract.

    /// Little-endian TIFF/EXIF block with orientation, DateTimeOriginal with
    /// offset, and GPS position plus accuracy.
    fn exif_block(orientation: u16, lat_dms: Dms, lon_dms: Dms) -> Vec<u8> {
        // Entry layout: tag(2) type(2) count(4) value-or-offset(4).
        const IFD0_ENTRIES: usize = 3;
        const EXIF_ENTRIES: usize = 2;
        const GPS_ENTRIES: usize = 5;
        let ifd0_at = 8usize;
        let ifd0_len = 2 + IFD0_ENTRIES * 12 + 4;
        let exif_at = ifd0_at + ifd0_len;
        let exif_len = 2 + EXIF_ENTRIES * 12 + 4;
        let gps_at = exif_at + exif_len;
        let gps_len = 2 + GPS_ENTRIES * 12 + 4;
        let data_at = gps_at + gps_len;

        let datetime = b"2026:10:28 07:42:00\0";
        let offset = b"+05:30\0";
        let dt_at = data_at;
        let off_at = dt_at + datetime.len();
        // Rationals are 4-byte aligned in practice; keep offsets even.
        let lat_at = (off_at + offset.len() + 1) & !1;
        let lon_at = lat_at + 24;
        let err_at = lon_at + 24;
        let data_end = err_at + 8;

        let mut data = vec![0u8; data_end - data_at];
        let put = |data: &mut Vec<u8>, at: usize, bytes: &[u8]| {
            data[at - data_at..at - data_at + bytes.len()].copy_from_slice(bytes);
        };
        put(&mut data, dt_at, datetime);
        put(&mut data, off_at, offset);
        for (i, (n, d)) in lat_dms.iter().enumerate() {
            put(&mut data, lat_at + i * 8, &n.to_le_bytes());
            put(&mut data, lat_at + i * 8 + 4, &d.to_le_bytes());
        }
        for (i, (n, d)) in lon_dms.iter().enumerate() {
            put(&mut data, lon_at + i * 8, &n.to_le_bytes());
            put(&mut data, lon_at + i * 8 + 4, &d.to_le_bytes());
        }
        // GPSHorizontalPositioningError = 12/1 metres.
        put(&mut data, err_at, &12u32.to_le_bytes());
        put(&mut data, err_at + 4, &1u32.to_le_bytes());

        let entry = |tag: u16, ty: u16, count: u32, value: u32| {
            let mut e = Vec::with_capacity(12);
            e.extend_from_slice(&tag.to_le_bytes());
            e.extend_from_slice(&ty.to_le_bytes());
            e.extend_from_slice(&count.to_le_bytes());
            e.extend_from_slice(&value.to_le_bytes());
            e
        };
        // A SHORT value sits in the low half of the 4-byte value slot.
        let short_entry = |tag: u16, value: u16| {
            let mut e = Vec::with_capacity(12);
            e.extend_from_slice(&tag.to_le_bytes());
            e.extend_from_slice(&3u16.to_le_bytes());
            e.extend_from_slice(&1u32.to_le_bytes());
            e.extend_from_slice(&value.to_le_bytes());
            e.extend_from_slice(&[0, 0]);
            e
        };

        let mut out = Vec::new();
        out.extend_from_slice(b"II");
        out.extend_from_slice(&42u16.to_le_bytes());
        out.extend_from_slice(&(ifd0_at as u32).to_le_bytes());

        // IFD0
        out.extend_from_slice(&(IFD0_ENTRIES as u16).to_le_bytes());
        out.extend_from_slice(&short_entry(0x0112, orientation)); // Orientation
        out.extend_from_slice(&entry(0x8769, 4, 1, exif_at as u32)); // ExifIFD
        out.extend_from_slice(&entry(0x8825, 4, 1, gps_at as u32)); // GPSInfoIFD
        out.extend_from_slice(&0u32.to_le_bytes()); // no IFD1

        // Exif IFD
        out.extend_from_slice(&(EXIF_ENTRIES as u16).to_le_bytes());
        out.extend_from_slice(&entry(0x9003, 2, datetime.len() as u32, dt_at as u32));
        out.extend_from_slice(&entry(0x9011, 2, offset.len() as u32, off_at as u32));
        out.extend_from_slice(&0u32.to_le_bytes());

        // GPS IFD
        out.extend_from_slice(&(GPS_ENTRIES as u16).to_le_bytes());
        let ref_entry = |tag: u16, letter: u8| {
            let mut e = Vec::with_capacity(12);
            e.extend_from_slice(&tag.to_le_bytes());
            e.extend_from_slice(&2u16.to_le_bytes());
            e.extend_from_slice(&2u32.to_le_bytes());
            e.extend_from_slice(&[letter, 0, 0, 0]);
            e
        };
        out.extend_from_slice(&ref_entry(0x0001, b'N')); // GPSLatitudeRef
        out.extend_from_slice(&entry(0x0002, 5, 3, lat_at as u32)); // GPSLatitude
        out.extend_from_slice(&ref_entry(0x0003, b'E')); // GPSLongitudeRef
        out.extend_from_slice(&entry(0x0004, 5, 3, lon_at as u32)); // GPSLongitude
        out.extend_from_slice(&entry(0x001F, 5, 1, err_at as u32)); // accuracy
        out.extend_from_slice(&0u32.to_le_bytes());

        out.extend_from_slice(&data);
        out
    }

    fn plain_jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut image = RgbImage::new(width, height);
        // Something non-uniform, so a rotation is observable.
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgb([(x * 7 % 256) as u8, (y * 11 % 256) as u8, 40]);
        }
        let mut out = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image)
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .unwrap();
        out.into_inner()
    }

    /// Splice an APP1 EXIF segment into a JPEG, right after SOI.
    fn with_exif(jpeg: &[u8], exif: &[u8]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(b"Exif\0\0");
        payload.extend_from_slice(exif);
        let length = (payload.len() + 2) as u16;

        let mut out = Vec::new();
        out.extend_from_slice(&jpeg[..2]); // SOI
        out.extend_from_slice(&[0xFF, 0xE1]);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    /// Degrees, minutes and seconds as three EXIF rationals.
    type Dms = [(u32, u32); 3];

    /// Bengaluru: 12.97712 N, 77.59581 E.
    fn bengaluru_dms() -> (Dms, Dms) {
        (
            [(12, 1), (58, 1), (37632, 1000)],
            [(77, 1), (35, 1), (44916, 1000)],
        )
    }

    fn gps_jpeg(orientation: u16) -> Vec<u8> {
        let (lat, lon) = bengaluru_dms();
        with_exif(&plain_jpeg(60, 40), &exif_block(orientation, lat, lon))
    }

    // ---- the fixture itself has to be real ----

    #[test]
    fn the_test_fixture_really_carries_gps_exif() {
        // If this fails, every privacy assertion below is vacuous.
        let exif = read_exif(&gps_jpeg(1));
        let location = exif.location.expect("fixture has no GPS");
        assert!((location.lat - 12.97712).abs() < 1e-6, "{location:?}");
        assert!((location.lon - 77.59581).abs() < 1e-6, "{location:?}");
        assert_eq!(location.accuracy_m, Some(12.0));
        assert_eq!(exif.datetime.as_deref(), Some("2026-10-28T07:42:00+05:30"));
        assert!(remaining_metadata(&gps_jpeg(1)).is_some());
    }

    #[test]
    fn reads_orientation() {
        assert_eq!(read_exif(&gps_jpeg(6)).orientation, Orientation(6));
        assert_eq!(read_exif(&gps_jpeg(1)).orientation, Orientation::NORMAL);
    }

    #[test]
    fn a_photo_without_exif_prefills_nothing() {
        let exif = read_exif(&plain_jpeg(10, 10));
        assert_eq!(exif, PhotoExif::default());
        assert!(exif.location.is_none());
        assert!(exif.datetime.is_none());
    }

    #[test]
    fn reads_southern_and_western_references() {
        let (lat, lon) = bengaluru_dms();
        let mut block = exif_block(1, lat, lon);
        // Flip GPSLatitudeRef to S and GPSLongitudeRef to W in place.
        let n_at = block.windows(4).position(|w| w == [b'N', 0, 0, 0]).unwrap();
        block[n_at] = b'S';
        let e_at = block.windows(4).position(|w| w == [b'E', 0, 0, 0]).unwrap();
        block[e_at] = b'W';
        let exif = read_exif(&with_exif(&plain_jpeg(20, 20), &block));
        let l = exif.location.unwrap();
        assert!(l.lat < 0.0, "{l:?}");
        assert!(l.lon < 0.0, "{l:?}");
    }

    // ---- the privacy guarantee ----

    #[test]
    fn cleaned_jpeg_has_no_gps_tags() {
        // SPEC section 5 step 2 asks for exactly this test.
        let cleaned = process(&gps_jpeg(1), 25.0).unwrap();
        assert!(read_exif(&cleaned.bytes).location.is_none());
        assert_eq!(remaining_metadata(&cleaned.bytes), None);
    }

    #[test]
    fn cleaned_jpeg_has_no_exif_block_at_all() {
        let cleaned = process(&gps_jpeg(1), 25.0).unwrap();
        assert!(
            exif::Reader::new()
                .read_from_container(&mut Cursor::new(&cleaned.bytes))
                .is_err(),
            "an EXIF block survived cleaning"
        );
        assert_eq!(read_exif(&cleaned.bytes), PhotoExif::default());
    }

    #[test]
    fn cleaned_jpeg_keeps_no_app1_segment() {
        let original = gps_jpeg(1);
        assert!(
            jpeg_segments(&original)
                .unwrap()
                .iter()
                .any(|(m, _)| *m == 0xE1),
            "fixture has no APP1 to remove"
        );
        let cleaned = process(&original, 25.0).unwrap();
        assert!(
            !jpeg_segments(&cleaned.bytes)
                .unwrap()
                .iter()
                .any(|(m, _)| is_metadata_segment(*m))
        );
    }

    #[test]
    fn cleaning_survives_a_rotated_photo() {
        // Orientation 6 forces the re-encode path, which must also come out
        // clean.
        let cleaned = process(&gps_jpeg(6), 25.0).unwrap();
        assert!(cleaned.reencoded);
        assert_eq!(remaining_metadata(&cleaned.bytes), None);
    }

    #[test]
    fn xmp_and_iptc_segments_are_removed() {
        // XMP rides in APP1 and IPTC in APP13; maker notes turn up across the
        // APPn range.
        let mut jpeg = plain_jpeg(20, 20);
        for marker in [0xE1u8, 0xED, 0xE3, 0xEF, 0xFE] {
            let payload = b"metadata-to-remove";
            let length = (payload.len() + 2) as u16;
            let mut spliced = Vec::new();
            spliced.extend_from_slice(&jpeg[..2]);
            spliced.extend_from_slice(&[0xFF, marker]);
            spliced.extend_from_slice(&length.to_be_bytes());
            spliced.extend_from_slice(payload);
            spliced.extend_from_slice(&jpeg[2..]);
            jpeg = spliced;
        }
        let cleaned = process(&jpeg, 25.0).unwrap();
        assert!(
            !cleaned
                .bytes
                .windows(18)
                .any(|w| w == b"metadata-to-remove"),
            "a metadata payload survived"
        );
        assert_eq!(remaining_metadata(&cleaned.bytes), None);
    }

    #[test]
    fn the_icc_profile_and_jfif_density_are_kept() {
        // Dropping APP2 would change how the photo looks, and neither it nor
        // APP0 is personal data.
        let jpeg = plain_jpeg(20, 20);
        let payload = b"ICC_PROFILE\0keep-me";
        let length = (payload.len() + 2) as u16;
        let mut with_icc = Vec::new();
        with_icc.extend_from_slice(&jpeg[..2]);
        with_icc.extend_from_slice(&[0xFF, 0xE2]);
        with_icc.extend_from_slice(&length.to_be_bytes());
        with_icc.extend_from_slice(payload);
        with_icc.extend_from_slice(&jpeg[2..]);

        let cleaned = process(&with_icc, 25.0).unwrap();
        assert!(
            cleaned.bytes.windows(7).any(|w| w == b"keep-me"),
            "the ICC profile was dropped"
        );
    }

    // ---- lossless where possible ----

    #[test]
    fn an_upright_jpeg_is_not_reencoded() {
        // Re-encoding every photo would degrade the ones that need nothing
        // but a metadata strip.
        let cleaned = process(&gps_jpeg(1), 25.0).unwrap();
        assert!(!cleaned.reencoded);
    }

    #[test]
    fn stripping_leaves_the_pixels_untouched() {
        // Build both from one base image, so the only difference going in is
        // the APP1 segment.
        let base = plain_jpeg(60, 40);
        let (lat, lon) = bengaluru_dms();
        let with_tags = with_exif(&base, &exif_block(1, lat, lon));

        let cleaned = process(&with_tags, 25.0).unwrap();
        assert!(!cleaned.reencoded, "took the re-encode path");
        // Same pixels, never decoded: the scan data must match the base byte
        // for byte.
        assert_eq!(jpeg_scan(&base), jpeg_scan(&cleaned.bytes));
    }

    fn jpeg_scan(bytes: &[u8]) -> Vec<u8> {
        let segments = jpeg_segments(bytes).unwrap();
        let (_, range) = segments.iter().find(|(m, _)| *m == 0xDA).unwrap();
        bytes[range.clone()].to_vec()
    }

    // ---- orientation is applied to the pixels ----

    #[test]
    fn rotation_swaps_the_dimensions() {
        // Orientation 6 is a 90 degree rotation, so a 60x40 photo comes out
        // 40x60 with the rotation baked in and the tag gone.
        let cleaned = process(&gps_jpeg(6), 25.0).unwrap();
        assert_eq!((cleaned.width, cleaned.height), (40, 60));
    }

    #[test]
    fn an_upright_photo_keeps_its_dimensions() {
        let cleaned = process(&gps_jpeg(1), 25.0).unwrap();
        assert_eq!((cleaned.width, cleaned.height), (60, 40));
    }

    #[test]
    fn every_orientation_is_handled() {
        for orientation in 1..=8u16 {
            let cleaned = process(&gps_jpeg(orientation), 25.0).unwrap();
            let expected = if Orientation(orientation).transposes() {
                (40, 60)
            } else {
                (60, 40)
            };
            assert_eq!(
                (cleaned.width, cleaned.height),
                expected,
                "orientation {orientation}"
            );
            assert_eq!(remaining_metadata(&cleaned.bytes), None);
        }
    }

    #[test]
    fn an_out_of_range_orientation_is_treated_as_upright() {
        let cleaned = process(&gps_jpeg(99), 25.0).unwrap();
        assert_eq!((cleaned.width, cleaned.height), (60, 40));
    }

    // ---- format handling ----

    #[test]
    fn sniffs_formats_from_content_not_extension() {
        assert_eq!(Format::sniff(&plain_jpeg(5, 5)), Some(Format::Jpeg));
        assert_eq!(Format::sniff(b"\x89PNG\r\n\x1a\nrest"), Some(Format::Png));
        assert_eq!(
            Format::sniff(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            Some(Format::Webp)
        );
        assert_eq!(
            Format::sniff(b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00"),
            Some(Format::Heic)
        );
        assert_eq!(Format::sniff(b"not an image at all"), None);
        assert_eq!(Format::sniff(b""), None);
    }

    #[test]
    fn png_round_trips_clean() {
        let mut image = RgbImage::new(16, 9);
        image.put_pixel(0, 0, image::Rgb([1, 2, 3]));
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let cleaned = process(&png.into_inner(), 25.0).unwrap();
        assert_eq!(cleaned.mime, "image/png");
        assert_eq!((cleaned.width, cleaned.height), (16, 9));
        assert_eq!(remaining_metadata(&cleaned.bytes), None);
    }

    #[test]
    fn rejects_an_unknown_format() {
        assert!(matches!(
            process(b"this is not an image", 25.0),
            Err(Error::UnknownFormat)
        ));
    }

    #[test]
    fn refuses_heic_with_a_reason() {
        // The contract allows image/heic, so "unknown format" would be
        // misleading; the honest answer is that decoding is unavailable.
        let heic = b"\x00\x00\x00\x18ftypheic\x00\x00\x00\x00rest-of-file";
        assert!(matches!(process(heic, 25.0), Err(Error::HeicUnsupported)));
    }

    #[test]
    fn rejects_a_file_over_the_limit() {
        let jpeg = plain_jpeg(200, 200);
        let limit_mb = (jpeg.len() as f64 - 1.0) / (1024.0 * 1024.0);
        assert!(matches!(
            process(&jpeg, limit_mb),
            Err(Error::TooLarge { .. })
        ));
        // And accepts it at exactly the limit.
        let exact = jpeg.len() as f64 / (1024.0 * 1024.0);
        assert!(process(&jpeg, exact).is_ok());
    }

    #[test]
    fn rejects_a_truncated_image() {
        let jpeg = plain_jpeg(40, 40);
        let truncated = &jpeg[..jpeg.len() / 3];
        assert!(process(truncated, 25.0).is_err());
    }

    // ---- the media entry ----

    #[test]
    fn sha256_is_of_the_uploaded_bytes() {
        let cleaned = process(&gps_jpeg(1), 25.0).unwrap();
        assert_eq!(cleaned.sha256, hex::encode(Sha256::digest(&cleaned.bytes)));
        // And not of the original, which is never uploaded.
        assert_ne!(cleaned.sha256, hex::encode(Sha256::digest(gps_jpeg(1))));
    }

    #[test]
    fn media_entry_matches_the_contract() {
        let cleaned = process(&gps_jpeg(1), 25.0).unwrap();
        let entry = cleaned.media_entry("logos:zb2rhPhotoA");
        // Check it through a submission, which is where a media entry lives.
        let mut submission: serde_json::Value =
            serde_json::from_str(include_str!("../../../examples/submission.json")).unwrap();
        submission["values"]["photos"] = serde_json::json!([entry]);
        assert!(
            crate::schema::is_valid(&submission),
            "{:?}",
            crate::schema::errors(&submission)
        );
    }

    // ---- prefills ----

    #[test]
    fn prefills_location_and_datetime_from_a_photo() {
        let exif = read_exif(&gps_jpeg(1));
        let (location, datetime) = prefills(&exif);
        let location = location.unwrap();
        assert_eq!(location["method"], serde_json::json!("photo"));
        assert_eq!(location["rounded"], serde_json::json!(false));
        assert_eq!(location["accuracy_m"], serde_json::json!(12.0));
        assert_eq!(datetime.as_deref(), Some("2026-10-28T07:42:00+05:30"));
    }

    #[test]
    fn a_photo_prefill_passes_field_validation() {
        use crate::fields::{self, Config, Field, FieldType, Method, Store};
        let exif = read_exif(&gps_jpeg(1));
        let (location, _) = prefills(&exif);

        let field = Field {
            key: "location".into(),
            ty: FieldType::Location,
            label: "Location".into(),
            help: None,
            required: true,
            config: Config::Location {
                methods: vec![Method::Photo],
                max_accuracy_m: Some(50.0),
                store: Store::Exact,
            },
        };
        let values = serde_json::json!({ "location": location.unwrap() });
        let report = fields::validate_values(
            &[field],
            &values,
            chrono::DateTime::parse_from_rfc3339("2026-10-28T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );
        assert!(report.is_ok(), "{report:?}");
    }

    #[test]
    fn a_rounded_field_still_rounds_a_photo_prefill() {
        use crate::fields::{self, Config, Field, FieldType, Method, Store};
        let exif = read_exif(&gps_jpeg(1));
        let (location, _) = prefills(&exif);
        let mut location = location.unwrap();

        let field = Field {
            key: "location".into(),
            ty: FieldType::Location,
            label: "Location".into(),
            help: None,
            required: true,
            config: Config::Location {
                methods: vec![Method::Photo],
                max_accuracy_m: None,
                store: Store::Rounded1km,
            },
        };
        // The exact point out of EXIF must not survive into the value.
        fields::apply_store_rule(&field, &mut location);
        assert_eq!(location["lat"], serde_json::json!(12.975));
        assert_eq!(location["rounded"], serde_json::json!(true));
    }

    #[test]
    fn datetime_without_an_offset_is_not_guessed() {
        // There is no correct offset to invent, and a datetime field needs one
        // to name an instant.
        let (lat, lon) = bengaluru_dms();
        let mut block = exif_block(1, lat, lon);
        // Blank the OffsetTimeOriginal tag id so it is not found.
        let tag_at = block
            .windows(2)
            .position(|w| w == 0x9011u16.to_le_bytes())
            .unwrap();
        block[tag_at] = 0x00;
        block[tag_at + 1] = 0x00;

        let exif = read_exif(&with_exif(&plain_jpeg(20, 20), &block));
        assert!(exif.datetime.is_none(), "{exif:?}");
        // The naive reading is still offered, for a UI that wants to ask.
        assert_eq!(exif.datetime_naive.as_deref(), Some("2026-10-28T07:42:00"));
    }

    // ---- WebP container handling ----

    #[test]
    fn strips_exif_and_xmp_chunks_from_a_webp() {
        // An extended-format WebP with VP8L pixels plus EXIF and XMP chunks.
        let mut body = Vec::new();
        body.extend_from_slice(b"WEBP");
        let mut chunk = |id: &[u8; 4], payload: &[u8]| {
            body.extend_from_slice(id);
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                body.push(0);
            }
        };
        chunk(b"VP8L", b"pixel-data");
        chunk(b"EXIF", b"gps-lives-here");
        chunk(b"XMP ", b"xmp-lives-here");

        let mut webp = Vec::new();
        webp.extend_from_slice(b"RIFF");
        webp.extend_from_slice(&(body.len() as u32).to_le_bytes());
        webp.extend_from_slice(&body);

        let stripped = strip_webp(&webp).expect("container should parse");
        assert!(!stripped.windows(14).any(|w| w == b"gps-lives-here"));
        assert!(!stripped.windows(14).any(|w| w == b"xmp-lives-here"));
        assert!(stripped.windows(10).any(|w| w == b"pixel-data"));
        // The RIFF size field must match the new body length.
        let declared = u32::from_le_bytes(stripped[4..8].try_into().unwrap()) as usize;
        assert_eq!(declared, stripped.len() - 8);
        assert_eq!(Format::sniff(&stripped), Some(Format::Webp));
    }

    #[test]
    fn a_malformed_container_does_not_strip_silently() {
        // Returning the input unchanged here would upload the metadata.
        assert!(strip_jpeg(b"not a jpeg").is_none());
        assert!(strip_webp(b"RIFF\x00\x00\x00\x00NOPE").is_none());
        // A JPEG whose segment length runs past the end of the file.
        let mut bad = vec![0xFF, 0xD8, 0xFF, 0xE1];
        bad.extend_from_slice(&9999u16.to_be_bytes());
        assert!(jpeg_segments(&bad).is_none());
        assert!(strip_jpeg(&bad).is_none());
    }

    #[test]
    fn remaining_metadata_reports_a_dirty_file() {
        assert!(remaining_metadata(&gps_jpeg(1)).is_some());
        assert_eq!(remaining_metadata(&plain_jpeg(10, 10)), None);
    }
}
