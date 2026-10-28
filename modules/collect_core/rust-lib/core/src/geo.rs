//! Location rounding.
//!
//! `CLAUDE.md`: rounding happens before anything is stored, on the submitter's
//! machine. All Collect data is public and nothing on Storage can be deleted,
//! so an exact coordinate written once is public forever — the rounding is the
//! only protection a `rounded_1km` protocol offers.

/// Cell size in degrees. A 0.01 degree cell is roughly 1.1 km north-south, and
/// narrower east-west the further you are from the equator.
pub const CELL_DEGREES: f64 = 0.01;

/// Highest latitude a cell centre may take, keeping it inside the schema's
/// -90..90 and away from the degenerate cell at the pole.
const MAX_LAT_CENTRE: f64 = 89.995;
const MAX_LON_CENTRE: f64 = 179.995;

/// Snap a coordinate to the centre of its 0.01 degree cell.
///
/// Returns the centre as (lat, lon). The exact point is discarded here and
/// never reaches a document.
pub fn round_1km(lat: f64, lon: f64) -> (f64, f64) {
    (
        clamp(round_axis(lat), MAX_LAT_CENTRE),
        clamp(round_axis(lon), MAX_LON_CENTRE),
    )
}

/// Is this coordinate already a cell centre?
///
/// Used when checking a received submission: a `rounded_1km` protocol whose
/// submission carries an off-centre coordinate has leaked precision, whatever
/// its `rounded` flag claims.
pub fn is_rounded(lat: f64, lon: f64) -> bool {
    let (rlat, rlon) = round_1km(lat, lon);
    // Compare the printed values rather than the bits: the submitter computed
    // theirs in their own build, and a one-ULP difference is not a leak.
    same_coordinate(lat, rlat) && same_coordinate(lon, rlon)
}

fn round_axis(v: f64) -> f64 {
    if !v.is_finite() {
        return 0.0;
    }
    // Work in hundredths so the floor lands on cell boundaries, then take the
    // centre. Dividing at the end keeps the result the shortest decimal that
    // round-trips, which is what canonical JSON will print.
    //
    // `v * 100.0` cannot be trusted at a boundary: 12.98 * 100.0 is a hair
    // below 1298, so a bare floor() files it under the cell beginning at
    // 12.97. Take that as a first guess, then settle it by comparing against
    // the boundaries themselves — a single division, so each is the closest
    // double to the real boundary.
    let mut cell = (v * 100.0).floor();
    if (cell + 1.0) / 100.0 <= v {
        cell += 1.0;
    } else if cell / 100.0 > v {
        cell -= 1.0;
    }
    (cell + 0.5) / 100.0
}

fn clamp(v: f64, limit: f64) -> f64 {
    v.clamp(-limit, limit)
}

/// Equality at the precision a cell centre carries. Half a cell is 0.005, so
/// a tolerance well below that cannot merge two different cells.
fn same_coordinate(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_to_cell_centre() {
        // The example submission's coordinates.
        let (lat, lon) = round_1km(12.97712, 77.59581);
        assert_eq!(lat, 12.975);
        assert_eq!(lon, 77.595);
    }

    #[test]
    fn centre_prints_as_a_short_decimal() {
        // The rounded value is signed as canonical JSON, so it must not print
        // as 12.975000000000001.
        let (lat, lon) = round_1km(12.97712, 77.59581);
        assert_eq!(
            crate::canonical::to_string(&serde_json::json!(lat)).unwrap(),
            "12.975"
        );
        assert_eq!(
            crate::canonical::to_string(&serde_json::json!(lon)).unwrap(),
            "77.595"
        );
    }

    #[test]
    fn every_point_in_a_cell_maps_to_one_centre() {
        let centre = round_1km(12.97, 77.59);
        for (lat, lon) in [
            (12.970_001, 77.590_001),
            (12.975, 77.595),
            (12.979_999, 77.599_999),
        ] {
            assert_eq!(round_1km(lat, lon), centre, "{lat},{lon}");
        }
    }

    #[test]
    fn adjacent_cells_stay_distinct() {
        assert_ne!(round_1km(12.97, 77.59), round_1km(12.98, 77.59));
        assert_ne!(round_1km(12.97, 77.59), round_1km(12.97, 77.60));
    }

    #[test]
    fn cell_boundary_belongs_to_the_higher_cell() {
        // floor() puts an exact boundary in the cell it opens.
        assert_eq!(round_1km(12.98, 0.0).0, 12.985);
    }

    #[test]
    fn handles_the_southern_and_western_hemispheres() {
        let (lat, lon) = round_1km(-33.8688, 151.2093);
        assert_eq!(lat, -33.865);
        assert_eq!(lon, 151.205);

        // A point just south of the equator must not round to a northern cell.
        assert_eq!(round_1km(-0.001, -0.001), (-0.005, -0.005));
        assert_eq!(round_1km(0.001, 0.001), (0.005, 0.005));
    }

    #[test]
    fn stays_inside_the_schema_range_at_the_extremes() {
        // Without clamping, 90.0 would land on a 90.005 centre, which the
        // schema's -90..90 bound rejects.
        let (lat, lon) = round_1km(90.0, 180.0);
        assert!((-90.0..=90.0).contains(&lat), "{lat}");
        assert!((-180.0..=180.0).contains(&lon), "{lon}");

        let (lat, lon) = round_1km(-90.0, -180.0);
        assert!((-90.0..=90.0).contains(&lat), "{lat}");
        assert!((-180.0..=180.0).contains(&lon), "{lon}");
    }

    #[test]
    fn rounding_is_idempotent() {
        let once = round_1km(12.97712, 77.59581);
        assert_eq!(round_1km(once.0, once.1), once);
    }

    #[test]
    fn recognises_a_rounded_coordinate() {
        let (lat, lon) = round_1km(12.97712, 77.59581);
        assert!(is_rounded(lat, lon));
    }

    #[test]
    fn recognises_an_unrounded_coordinate() {
        // The whole point of the check: this is what a leaking submission
        // looks like.
        assert!(!is_rounded(12.97712, 77.59581));
        assert!(!is_rounded(12.975, 77.59581));
        assert!(!is_rounded(12.97712, 77.595));
    }

    #[test]
    fn centre_is_always_within_half_a_cell_of_the_input() {
        // The property that matters, swept across boundaries rather than at
        // hand-picked points: this is where the `v * 100.0` rounding bug hid.
        let mut checked = 0;
        for i in -18_000i32..=18_000 {
            let v = f64::from(i) / 100.0;
            for offset in [0.0, 0.001, 0.005, 0.009, 0.0099] {
                let point = v + offset;
                if !(-180.0..=180.0).contains(&point) {
                    continue;
                }
                let (_, centre) = round_1km(0.0, point);
                let delta = (point - centre).abs();
                assert!(
                    delta <= CELL_DEGREES / 2.0 + 1e-9,
                    "{point} rounded to {centre}, off by {delta}"
                );
                checked += 1;
            }
        }
        assert!(checked > 100_000, "swept only {checked} points");
    }

    #[test]
    fn centres_are_monotonic_across_cells() {
        // Two points in increasing order must never round backwards.
        let mut previous = f64::NEG_INFINITY;
        for i in -9_000i32..=9_000 {
            let (centre, _) = round_1km(f64::from(i) / 100.0, 0.0);
            assert!(centre >= previous, "{centre} followed {previous}");
            previous = centre;
        }
    }

    #[test]
    fn non_finite_input_does_not_escape() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let (lat, lon) = round_1km(bad, bad);
            assert!(lat.is_finite() && lon.is_finite(), "{bad}");
        }
    }
}
