//! Token rewards for approved submissions (SPEC section 11).
//!
//! An experiment may offer a reward in LEZ TestToken per approved submission.
//! After the experiment ends, the creator reviews a payout plan and confirms
//! it; each submitter is then paid with one public transfer through the
//! `lez_core` module's wallet — the same wallet the person opens in LEZ
//! Wallet. Nothing here pays on its own: a plan is only ever carried out one
//! confirmed transfer at a time.
//!
//! What `lez_core` 0.5.1 takes, checked against the installed binary:
//! `transfer_public(from_hex, to_hex, amount_le16_hex) -> tstr` and
//! `poll_transaction_status(tx_hash_hex) -> bool`. Accounts are 32-byte ids
//! in hex (the wallet shows them in base58); amounts are lepta (10^9 per
//! token) as a 16-byte little-endian hex number.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The only token a reward can be in, as written into the manifest.
pub const TOKEN: &str = "lez:testnet";
/// What the wallet calls it.
pub const SYMBOL: &str = "TestToken";
/// Lepta per token.
pub const DECIMALS: u32 = 9;
/// No submitter can be paid for more submissions than this, whatever the
/// manifest says: one person filing a form a thousand times is not a
/// thousand rewards.
pub const MAX_PER_SUBMITTER_CEILING: u64 = 1000;

/// A decimal token amount ("0.0005") as lepta. At most nine decimals, never
/// negative, never zero — a reward of nothing is no reward.
pub fn parse_tokens(text: &str) -> Result<u128, String> {
    let text = text.trim();
    let (whole, frac) = match text.split_once('.') {
        Some((w, f)) => (w, f),
        None => (text, ""),
    };
    let whole = if whole.is_empty() { "0" } else { whole };
    if !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return Err("Write the amount as a number, like 0.0005".into());
    }
    if frac.len() > DECIMALS as usize {
        return Err(format!("At most {DECIMALS} decimal places"));
    }
    let scale = 10u128.pow(DECIMALS);
    let whole: u128 = whole.parse().map_err(|_| "That amount is too large".to_string())?;
    let frac: u128 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<9}").parse().map_err(|_| "Write the amount as a number".to_string())?
    };
    let lepta = whole
        .checked_mul(scale)
        .and_then(|w| w.checked_add(frac))
        .ok_or_else(|| "That amount is too large".to_string())?;
    if lepta == 0 {
        return Err("A reward has to be more than zero".into());
    }
    Ok(lepta)
}

/// Lepta as a decimal token amount, trailing zeros trimmed: 500000 → "0.0005".
pub fn format_tokens(lepta: u128) -> String {
    let scale = 10u128.pow(DECIMALS);
    let whole = lepta / scale;
    let frac = lepta % scale;
    if frac == 0 {
        return whole.to_string();
    }
    let frac = format!("{frac:09}");
    format!("{whole}.{}", frac.trim_end_matches('0'))
}

/// The amount as `lez_core` takes it: 16 bytes, little-endian, hex.
pub fn le16_hex(lepta: u128) -> String {
    hex::encode(lepta.to_le_bytes())
}

/// A balance as `lez_core` reports it. The interface says only `tstr`, so
/// both plausible encodings are read: a decimal lepta figure (what the
/// wallet UI treats it as) or the 16-byte little-endian hex `transfer_*`
/// takes.
pub fn parse_balance(text: &str) -> Option<u128> {
    let text = text.trim().trim_matches('"');
    if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit()) {
        return text.parse().ok();
    }
    let bytes = hex::decode(text.trim_start_matches("0x")).ok()?;
    if bytes.len() != 16 {
        return None;
    }
    let mut le = [0u8; 16];
    le.copy_from_slice(&bytes);
    Some(u128::from_le_bytes(le))
}

const BASE58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// A LEZ public account address, as someone would paste it, in the hex
/// `lez_core` wants. Takes what the wallet shows (base58, optionally with a
/// `Public/` prefix) or 64 hex digits. Anything that is not 32 bytes is not
/// an account.
pub fn account_hex(address: &str) -> Result<String, String> {
    let address = address.trim();
    let address = address
        .strip_prefix("Public/")
        .or_else(|| address.strip_prefix("public/"))
        .unwrap_or(address);
    let plain = address.trim_start_matches("0x");
    if plain.len() == 64 && plain.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(plain.to_ascii_lowercase());
    }
    let bytes = base58_decode(address)
        .ok_or_else(|| "That is not a LEZ account address".to_string())?;
    if bytes.len() != 32 {
        return Err("That is not a LEZ public account address (it should be 32 bytes)".into());
    }
    Ok(hex::encode(bytes))
}

fn base58_decode(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() {
        return None;
    }
    let zeros = text.bytes().take_while(|&b| b == b'1').count();
    // Base-256 digits, least significant first.
    let mut digits: Vec<u8> = Vec::new();
    for c in text.bytes() {
        let mut carry = BASE58.iter().position(|&a| a == c)? as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) * 58;
            *d = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            digits.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let mut out = vec![0u8; zeros];
    out.extend(digits.iter().rev());
    Some(out)
}

/// The reward block a manifest carries.
pub fn manifest_reward(lepta: u128, max_per_submitter: u64) -> Value {
    json!({
        "token": TOKEN,
        "amount": lepta.to_string(),
        "max_per_submitter": max_per_submitter,
    })
}

/// A manifest's reward, read back: (lepta per submission, cap per submitter).
pub fn read_reward(experiment: &Value) -> Option<(u128, u64)> {
    let reward = experiment.get("reward")?;
    if reward.get("token").and_then(Value::as_str) != Some(TOKEN) {
        return None;
    }
    let lepta: u128 = reward.get("amount").and_then(Value::as_str)?.parse().ok()?;
    let cap = reward
        .get("max_per_submitter")
        .and_then(Value::as_u64)?
        .min(MAX_PER_SUBMITTER_CEILING);
    Some((lepta, cap))
}

/// One row of what the creator is asked to confirm.
pub struct Payee {
    pub submitter: String,
    pub name: String,
    pub wallet: String,
    pub wallet_hex: String,
    pub cids: Vec<String>,
    pub lepta: u128,
}

impl Payee {
    pub fn as_json(&self) -> Value {
        json!({
            "submitter": self.submitter,
            "name": self.name,
            "wallet": self.wallet,
            "wallet_hex": self.wallet_hex,
            "submissions": self.cids,
            "count": self.cids.len(),
            "amount_lepta": self.lepta.to_string(),
            "amount": format_tokens(self.lepta),
        })
    }
}

/// A stable name for a set of payees, so a payment can insist the plan it
/// is carrying out is the one the creator saw. Any change — a new approval,
/// a payment that landed in between — changes it.
pub fn plan_id(payees: &[Payee]) -> String {
    let mut h = Sha256::new();
    for p in payees {
        h.update(p.submitter.as_bytes());
        h.update(b"\0");
        h.update(p.wallet_hex.as_bytes());
        h.update(b"\0");
        h.update(p.lepta.to_string().as_bytes());
        for cid in &p.cids {
            h.update(b"\0");
            h.update(cid.as_bytes());
        }
        h.update(b"\n");
    }
    hex::encode(&h.finalize()[..12])
}

/// Somewhere rewards can be paid from: the `lez_core` wallet, or a memory
/// one in tests.
pub trait Ledger: Send {
    /// Public balance of `account_hex`, in lepta.
    fn balance(&self, account_hex: &str) -> Result<u128, String>;
    /// Send `lepta` publicly; the transaction hash on success.
    fn transfer_public(&self, from_hex: &str, to_hex: &str, lepta: u128) -> Result<String, String>;
    /// Whether the transaction is in a block yet.
    fn confirmed(&self, tx: &str) -> Result<bool, String>;
    fn describe(&self) -> String {
        "a ledger".into()
    }
}

/// An in-process ledger, for tests: balances by account, every transfer
/// recorded, and a switch to make the next transfer fail.
#[derive(Default)]
pub struct MemoryLedger {
    pub balances: std::sync::Mutex<std::collections::BTreeMap<String, u128>>,
    pub sent: std::sync::Mutex<Vec<(String, String, u128)>>,
    pub fail_next: std::sync::Mutex<Option<String>>,
}

impl Ledger for MemoryLedger {
    fn balance(&self, account_hex: &str) -> Result<u128, String> {
        Ok(*self.balances.lock().unwrap().get(account_hex).unwrap_or(&0))
    }

    fn transfer_public(&self, from_hex: &str, to_hex: &str, lepta: u128) -> Result<String, String> {
        if let Some(e) = self.fail_next.lock().unwrap().take() {
            return Err(e);
        }
        let mut balances = self.balances.lock().unwrap();
        let from = balances.get(from_hex).copied().unwrap_or(0);
        if from < lepta {
            return Err("insufficient balance".into());
        }
        balances.insert(from_hex.to_string(), from - lepta);
        *balances.entry(to_hex.to_string()).or_insert(0) += lepta;
        let mut sent = self.sent.lock().unwrap();
        sent.push((from_hex.to_string(), to_hex.to_string(), lepta));
        Ok(format!("{:064x}", sent.len()))
    }

    fn confirmed(&self, _tx: &str) -> Result<bool, String> {
        Ok(true)
    }
}

impl<T: Ledger + Sync> Ledger for std::sync::Arc<T> {
    fn balance(&self, a: &str) -> Result<u128, String> {
        (**self).balance(a)
    }
    fn transfer_public(&self, f: &str, t: &str, l: u128) -> Result<String, String> {
        (**self).transfer_public(f, t, l)
    }
    fn confirmed(&self, tx: &str) -> Result<bool, String> {
        (**self).confirmed(tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_amounts_round_trip_exactly() {
        assert_eq!(parse_tokens("0.0005").unwrap(), 500_000);
        assert_eq!(parse_tokens("1").unwrap(), 1_000_000_000);
        assert_eq!(parse_tokens(".5").unwrap(), 500_000_000);
        assert_eq!(parse_tokens("0.000000001").unwrap(), 1);
        assert_eq!(format_tokens(500_000), "0.0005");
        assert_eq!(format_tokens(1_500_000_000), "1.5");
        assert_eq!(format_tokens(2_000_000_000), "2");
        assert!(parse_tokens("0").is_err());
        assert!(parse_tokens("0.0000000001").is_err(), "ten decimals");
        assert!(parse_tokens("-1").is_err());
        assert!(parse_tokens("1,5").is_err());
        assert!(parse_tokens("abc").is_err());
    }

    #[test]
    fn amounts_go_to_lez_core_as_16_byte_little_endian_hex() {
        assert_eq!(le16_hex(500_000), "20a10700000000000000000000000000");
        assert_eq!(parse_balance("20a10700000000000000000000000000"), Some(500_000));
        assert_eq!(parse_balance("500000"), Some(500_000));
        assert_eq!(parse_balance("\"1000\""), Some(1000));
        assert_eq!(parse_balance("not a number"), None);
    }

    #[test]
    fn addresses_are_read_in_either_form_and_must_be_32_bytes() {
        let hex = "0".repeat(62) + "ff";
        assert_eq!(account_hex(&hex).unwrap(), hex);
        // The same account in base58, as the wallet shows it.
        let b58 = {
            // 31 zero bytes then 0xff: thirty-one '1's then 255 in base58.
            let mut s = "1".repeat(31);
            s.push_str("5Q");
            s
        };
        assert_eq!(account_hex(&b58).unwrap(), hex);
        assert_eq!(account_hex(&format!("Public/{b58}")).unwrap(), hex);
        assert!(account_hex("lez1qexamplewalletaddress").is_err(), "'l' is not base58");
        assert!(account_hex("5Q").is_err(), "one byte is not an account");
        assert!(account_hex("").is_err());
    }

    #[test]
    fn a_plan_id_changes_with_anything_that_changes_who_is_paid() {
        let payee = |cids: &[&str]| Payee {
            submitter: "aa".into(),
            name: String::new(),
            wallet: "w".into(),
            wallet_hex: "ff".into(),
            cids: cids.iter().map(|c| c.to_string()).collect(),
            lepta: 500_000 * cids.len() as u128,
        };
        let one = plan_id(&[payee(&["logos:a"])]);
        assert_eq!(one, plan_id(&[payee(&["logos:a"])]));
        assert_ne!(one, plan_id(&[payee(&["logos:a", "logos:b"])]));
        assert_ne!(one, plan_id(&[]));
    }

    #[test]
    fn the_memory_ledger_refuses_to_overdraw() {
        let l = MemoryLedger::default();
        l.balances.lock().unwrap().insert("a".into(), 10);
        assert!(l.transfer_public("a", "b", 11).is_err());
        assert!(l.transfer_public("a", "b", 10).is_ok());
        assert_eq!(l.balance("b").unwrap(), 10);
    }
}
