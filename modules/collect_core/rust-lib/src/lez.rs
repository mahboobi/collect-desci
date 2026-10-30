//! The real LEZ wallet, reached through the `lez_core` module.
//!
//! Basecamp runs one `lez_core` and so one wallet handle: the wallet the
//! person opens in LEZ Wallet is the one this pays from. Nothing here opens,
//! creates or unlocks a wallet, and no key ever passes through Collect — if
//! the wallet is not open, the call fails and the screen says to open it.
//!
//! `lez_core` is called by name and deliberately **not** declared in
//! metadata.json: a declared dependency that is not installed stops Collect
//! opening at all (CLAUDE.md 8b), and rewards are optional. The cost is that
//! it only answers when something else has loaded it — LEZ Wallet being open
//! does that.
//!
//! The calls, from `lez_core` 0.5.1's interface and checked against the
//! installed binary:
//!   get_balance(account_id_hex, is_public) -> tstr
//!   transfer_public(from_hex, to_hex, amount_le16_hex) -> tstr
//!   poll_transaction_status(tx_hash_hex) -> bool

use collect_core_logic::rewards::{le16_hex, parse_balance, Ledger};
use logos_rust_sdk::{LogosModuleSDK, PluginProxy};
use serde_json::{json, Value};

const LEZ_MODULE: &str = "lez_core";

pub struct LezLedger {
    proxy: PluginProxy,
}

impl LezLedger {
    pub fn connect() -> Self {
        LezLedger { proxy: LogosModuleSDK::new().plugin(LEZ_MODULE) }
    }

    fn call(&self, method: &str, args: Value) -> Result<Value, String> {
        let answer = self
            .proxy
            .call_json(method, &args)
            .map_err(|e| format!("{method}: {e} (is LEZ Wallet open?)"))?;
        unwrap_result(&answer).map_err(|e| format!("{method}: {e}"))
    }
}

/// A `StdLogosResult`'s value, or its error. Plain values pass through.
fn unwrap_result(value: &Value) -> Result<Value, String> {
    match value {
        Value::Object(map) if map.contains_key("success") => {
            if map.get("success").and_then(Value::as_bool) == Some(false) {
                return Err(map
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("lez_core refused the call")
                    .to_string());
            }
            Ok(map.get("value").cloned().unwrap_or(Value::Null))
        }
        other => Ok(other.clone()),
    }
}

/// Text out of a reply, whether it came as a string or as JSON.
fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// What a transfer returned, as a transaction hash: the reply itself, or the
/// hash inside it if it is JSON naming one. An empty reply is a failure — a
/// transfer with no hash cannot be followed up.
fn tx_hash(reply: &Value) -> Result<String, String> {
    let raw = text(reply);
    let inner: Option<Value> = serde_json::from_str(&raw).ok();
    let hash = match inner.as_ref() {
        Some(Value::Object(map)) => {
            if let Some(e) = map.get("error").and_then(Value::as_str) {
                return Err(e.to_string());
            }
            ["tx_hash", "txHash", "hash", "value"]
                .iter()
                .find_map(|k| map.get(*k).and_then(Value::as_str))
                .unwrap_or("")
                .to_string()
        }
        Some(Value::String(s)) => s.clone(),
        _ => raw.trim().to_string(),
    };
    if hash.is_empty() {
        return Err("lez_core returned no transaction hash".into());
    }
    Ok(hash)
}

impl Ledger for LezLedger {
    fn balance(&self, account_hex: &str) -> Result<u128, String> {
        let reply = self.call("get_balance", json!([account_hex, true]))?;
        let t = text(&reply);
        parse_balance(&t).ok_or_else(|| format!("unreadable balance `{t}`"))
    }

    fn transfer_public(&self, from_hex: &str, to_hex: &str, lepta: u128) -> Result<String, String> {
        let reply = self.call("transfer_public", json!([from_hex, to_hex, le16_hex(lepta)]))?;
        tx_hash(&reply)
    }

    fn confirmed(&self, tx: &str) -> Result<bool, String> {
        let reply = self.call("poll_transaction_status", json!([tx]))?;
        Ok(match reply {
            Value::Bool(b) => b,
            other => text(&other).trim() == "true",
        })
    }

    fn describe(&self) -> String {
        format!("the LEZ wallet via `{LEZ_MODULE}`")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transfer_reply_yields_its_hash_in_any_plausible_shape() {
        assert_eq!(tx_hash(&json!("abc123")).unwrap(), "abc123");
        assert_eq!(tx_hash(&json!("\"abc123\"")).unwrap(), "abc123");
        assert_eq!(tx_hash(&json!(r#"{"tx_hash":"abc"}"#)).unwrap(), "abc");
        assert!(tx_hash(&json!(r#"{"error":"insufficient balance"}"#)).is_err());
        assert!(tx_hash(&json!("")).is_err());
    }

    #[test]
    fn a_failed_logos_result_is_an_error_not_a_value() {
        let failed = json!({ "success": false, "error": "wallet not open" });
        assert_eq!(unwrap_result(&failed).unwrap_err(), "wallet not open");
        assert_eq!(unwrap_result(&json!({ "success": true, "value": "1" })).unwrap(), json!("1"));
        assert_eq!(unwrap_result(&json!("plain")).unwrap(), json!("plain"));
    }
}
