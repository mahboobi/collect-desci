//! Discussion and profiles among a project's members.
//!
//! Both travel on the experiment's own Delivery topic, as two more signed
//! announcement kinds (`message`, `profile`), so they inherit everything the
//! receive path already does: deduplication by `messageHash`, the topic
//! agreeing with the announcement, the contract, then the signature.
//!
//! **Nothing here is ever written to Logos Storage.** Messages are kept in a
//! local history file per experiment; Delivery's Store fills in what was
//! missed while offline, for as long as the operator keeps it. Messages are
//! readable by anyone on the topic. Profiles are not: each shared field is
//! sealed per recipient (`seal`).
//!
//! ## Who is a member
//!
//! Anyone whose signed `message` or `profile` announcement arrived on the
//! topic. That is how a member's box key becomes known, and a member can only
//! be sealed for once their box key is known. So a node introduces itself
//! with a `profile` announcement as soon as it can — with no sealed entries
//! if it shares nothing — and re-sends whenever a new member appears or its
//! own shared fields change.

use crate::profile::{self, Profile};
use crate::seal::BoxKey;
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The most members one `profile` announcement is sealed for. The contract
/// caps `sealed` at 32; more recent members win.
pub const MAX_RECIPIENTS: usize = 32;

/// Kept per experiment, newest last. Older ones are dropped from memory and
/// from the file on the next rewrite.
const HISTORY_CAP: usize = 2000;

/// The longest a message may be, in characters. Mirrors the contract.
pub const MAX_TEXT: usize = 2000;

/// Re-sending a profile because a new member appeared waits at least this
/// long since the last send, so a burst of joins costs one message. A change
/// to one's own profile does not wait.
const RESHARE_GAP_SECS: i64 = 60;

const MEMBERS_FILE: &str = "members.json";
const MESSAGES_DIR: &str = "messages";

/// What became of an incoming `message` or `profile`.
#[derive(Debug, PartialEq)]
pub enum Received {
    /// Stored. Carries what the UI shows.
    Kept(Value),
    /// Already had it (same sender and id), or our own echoed back.
    Duplicate,
    /// A profile with nothing sealed for us. The member is still recorded.
    NothingForUs,
    /// Refused, with why.
    Refused(String),
}

#[derive(Default)]
struct Shared {
    /// Box keys of the members last sealed for.
    to: BTreeSet<String>,
    /// The profile generation last sent.
    generation: u64,
    at: Option<DateTime<Utc>>,
    /// Whether we have introduced ourselves on this topic at all.
    introduced: bool,
}

pub struct Social {
    profile: Profile,
    box_key: Option<BoxKey>,
    dir: Option<PathBuf>,
    /// Bumped on every change to one's own profile, so each experiment knows
    /// whether what it last sent is stale.
    generation: u64,
    /// experiment -> member key -> { box, profile, first_seen, last_seen }
    members: BTreeMap<String, BTreeMap<String, Value>>,
    /// experiment -> messages, oldest first. Loaded on first use.
    messages: BTreeMap<String, Vec<Value>>,
    shared: BTreeMap<String, Shared>,
}

impl Default for Social {
    fn default() -> Self {
        Social {
            profile: Profile::default(),
            box_key: None,
            dir: None,
            generation: 1,
            members: BTreeMap::new(),
            messages: BTreeMap::new(),
            shared: BTreeMap::new(),
        }
    }
}

impl Social {
    /// Open this installation's profile, box key and member list.
    ///
    /// A profile that cannot be read is reported and left alone — never
    /// overwritten — and the rest still attaches.
    pub fn attach(&mut self, dir: &Path) -> Result<(), String> {
        self.dir = Some(dir.to_path_buf());
        self.box_key = Some(BoxKey::load_or_create(dir).map_err(|e| e.to_string())?);
        if let Ok(bytes) = std::fs::read(dir.join(MEMBERS_FILE)) {
            if let Ok(Value::Object(m)) = serde_json::from_slice::<Value>(&bytes) {
                for (exp, members) in m {
                    if let Value::Object(members) = members {
                        self.members.insert(exp, members.into_iter().collect());
                    }
                }
            }
        }
        self.profile = Profile::load(dir).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// A box key for a node that was never given a directory: tests, and
    /// fixtures mode before the context arrives.
    pub fn ensure_ephemeral_key(&mut self) {
        if self.box_key.is_none() {
            self.box_key = Some(BoxKey::ephemeral());
        }
    }

    pub fn box_public(&self) -> Option<String> {
        self.box_key.as_ref().map(BoxKey::public_hex)
    }

    // ------------------------------------------------------------- profile

    pub fn profile_json(&self) -> Value {
        json!({ "fields": self.profile.as_json(), "sex_options": profile::SEX_OPTIONS })
    }

    /// `profile.save`. All-or-nothing; problems come back keyed by field.
    /// This person's own wallet address, as saved in their profile, whether
    /// or not it is shared with members. Read here, never taken from the
    /// UI, so a submission can only ever carry the address the person saved.
    pub fn own_wallet(&self) -> Option<String> {
        self.profile
            .as_json()
            .pointer("/wallet/value")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(str::to_string)
    }

    pub fn save_profile(&mut self, request: &Value) -> Result<Value, Value> {
        self.profile
            .update(request)
            .map_err(|p| json!({ "problems": p }))?;
        self.profile
            .save()
            .map_err(|e| json!({ "message": e.to_string() }))?;
        self.generation += 1;
        Ok(self.profile_json())
    }

    /// `profile.setPhoto`, from the bytes of a picked file.
    pub fn set_photo(&mut self, bytes: &[u8], share: bool) -> Result<Value, String> {
        self.profile.set_photo(bytes, share).map_err(|e| e.to_string())?;
        self.profile.save().map_err(|e| e.to_string())?;
        self.generation += 1;
        Ok(self.profile_json())
    }

    pub fn clear_photo(&mut self) -> Result<Value, String> {
        self.profile.clear_photo();
        self.profile.save().map_err(|e| e.to_string())?;
        self.generation += 1;
        Ok(self.profile_json())
    }

    // ------------------------------------------------------------ messages

    /// An unsigned `message` announcement, or why not.
    pub fn compose(
        &self,
        experiment: &str,
        me: &str,
        text: &str,
        reply_to: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Value, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("A message cannot be empty".into());
        }
        if text.chars().count() > MAX_TEXT {
            return Err(format!("A message is at most {MAX_TEXT} characters"));
        }
        let Some(box_key) = self.box_public() else {
            return Err("no box key yet: the module context is not ready".into());
        };
        let mut m = json!({
            "kind": "message",
            "experiment": experiment,
            "from": me,
            "at": now.to_rfc3339(),
            "id": uuid::Uuid::now_v7().to_string(),
            "text": text,
            "box": box_key,
            "sig": "00".repeat(64),
        });
        if let Some(r) = reply_to.filter(|r| !r.is_empty()) {
            m["reply_to"] = json!(r);
        }
        Ok(m)
    }

    /// Keep a message — ours once sent, or one that arrived. The caller has
    /// already checked the contract and the signature.
    pub fn receive_message(&mut self, experiment: &str, signed: &Value, me: &str, now: DateTime<Utc>) -> Received {
        let from = signed.get("from").and_then(Value::as_str).unwrap_or("");
        let id = signed.get("id").and_then(Value::as_str).unwrap_or("");
        if from != me {
            self.note_member(experiment, from, signed.get("box").and_then(Value::as_str), now);
        }
        let history = self.history(experiment);
        if history.iter().any(|m| m["from"] == json!(from) && m["id"] == json!(id)) {
            return Received::Duplicate;
        }
        let kept = json!({
            "id": id,
            "from": from,
            "at": signed.get("at").cloned().unwrap_or(Value::Null),
            "text": signed.get("text").cloned().unwrap_or(Value::Null),
            "reply_to": signed.get("reply_to").cloned().unwrap_or(Value::Null),
        });
        history.push(kept.clone());
        // Arrival order is not send order (Store catch-up delivers late), so
        // keep the history sorted by the sender's own timestamp.
        history.sort_by(|a, b| a["at"].as_str().unwrap_or("").cmp(b["at"].as_str().unwrap_or("")));
        let overflow = history.len().saturating_sub(HISTORY_CAP);
        if overflow > 0 {
            history.drain(..overflow);
            self.rewrite_history(experiment);
        } else {
            self.append_history(experiment, &kept);
        }
        Received::Kept(self.present(experiment, &kept, me))
    }

    /// `messages.list`: the history, each message labelled with what we know
    /// of its sender.
    pub fn list_messages(&mut self, experiment: &str, me: &str) -> Value {
        let msgs: Vec<Value> = self.history(experiment).clone();
        let out: Vec<Value> = msgs.iter().map(|m| self.present(experiment, m, me)).collect();
        json!({ "experiment": experiment, "messages": out })
    }

    fn present(&self, experiment: &str, m: &Value, me: &str) -> Value {
        let from = m["from"].as_str().unwrap_or("");
        let mine = from == me;
        let profile = if mine {
            None
        } else {
            self.members
                .get(experiment)
                .and_then(|ms| ms.get(from))
                .and_then(|member| member.get("profile"))
                .filter(|p| p.is_object())
        };
        let name = if mine {
            self.profile.alias()
        } else {
            profile.and_then(|p| p.get("alias")).and_then(Value::as_str).map(str::to_string)
        };
        let mut out = m.clone();
        out["mine"] = json!(mine);
        out["name"] = name.map(Value::from).unwrap_or(Value::Null);
        out["short"] = json!(from.chars().take(8).collect::<String>());
        out["photo"] = profile
            .and_then(|p| p.get("photo"))
            .cloned()
            .unwrap_or(Value::Null);
        out
    }

    // ------------------------------------------------------------ profiles

    /// Should we send a `profile` announcement on this experiment now?
    pub fn needs_share(&self, experiment: &str, me: &str, now: DateTime<Utc>) -> bool {
        let state = self.shared.get(experiment);
        let introduced = state.map(|s| s.introduced).unwrap_or(false);
        if !introduced {
            return true;
        }
        let state = state.expect("introduced implies state");
        let has_shared = self.profile.shared().is_some();
        if state.generation != self.generation {
            return true;
        }
        if !has_shared {
            // Nothing to seal: a new member learns our box key from our
            // introduction or our messages.
            return false;
        }
        let current: BTreeSet<String> = self.recipients(experiment, me).into_iter().map(|(_, b)| b).collect();
        if current.is_subset(&state.to) {
            return false;
        }
        state
            .at
            .map(|at| now - at >= Duration::seconds(RESHARE_GAP_SECS))
            .unwrap_or(true)
    }

    /// An unsigned `profile` announcement: our shared fields sealed for each
    /// known member, or an empty introduction.
    pub fn compose_profile(&self, experiment: &str, me: &str, now: DateTime<Utc>) -> Result<Value, String> {
        let Some(key) = self.box_key.as_ref() else {
            return Err("no box key yet: the module context is not ready".into());
        };
        let mut sealed = Vec::new();
        if let Some(shared) = self.profile.shared() {
            let plain = serde_json::to_vec(&shared).map_err(|e| e.to_string())?;
            for (to, to_box) in self.recipients(experiment, me) {
                match key.seal(experiment, me, &to, &to_box, &plain) {
                    Ok(entry) => sealed.push(entry),
                    // One member with an unusable key should not stop the
                    // others getting the profile.
                    Err(e) => eprintln!("collect_core: cannot seal for {to}: {e}"),
                }
            }
        }
        Ok(json!({
            "kind": "profile",
            "experiment": experiment,
            "from": me,
            "at": now.to_rfc3339(),
            "box": key.public_hex(),
            "sealed": sealed,
            "sig": "00".repeat(64),
        }))
    }

    /// Record that a `profile` announcement for this experiment went out.
    pub fn mark_shared(&mut self, experiment: &str, me: &str, now: DateTime<Utc>) {
        let to = if self.profile.shared().is_some() {
            self.recipients(experiment, me).into_iter().map(|(_, b)| b).collect()
        } else {
            BTreeSet::new()
        };
        self.shared.insert(
            experiment.to_string(),
            Shared { to, generation: self.generation, at: Some(now), introduced: true },
        );
    }

    /// Take a `profile` announcement that arrived. Contract and signature are
    /// already checked by the caller.
    pub fn receive_profile(&mut self, experiment: &str, signed: &Value, me: &str, now: DateTime<Utc>) -> Received {
        let from = signed.get("from").and_then(Value::as_str).unwrap_or("").to_string();
        if from == me {
            return Received::Duplicate;
        }
        let Some(from_box) = signed.get("box").and_then(Value::as_str).map(str::to_string) else {
            return Received::Refused("no box key".into());
        };
        self.note_member(experiment, &from, Some(&from_box), now);
        let entry = signed
            .get("sealed")
            .and_then(Value::as_array)
            .and_then(|s| s.iter().find(|e| e.get("to").and_then(Value::as_str) == Some(me)))
            .cloned();
        let Some(entry) = entry else {
            return Received::NothingForUs;
        };
        let Some(key) = self.box_key.as_ref() else {
            return Received::Refused("no box key of our own yet".into());
        };
        let plain = match key.open(experiment, &from, &from_box, me, &entry) {
            Ok(p) => p,
            Err(e) => return Received::Refused(e.to_string()),
        };
        let Some(accepted) = serde_json::from_slice::<Value>(&plain)
            .ok()
            .and_then(|v| profile::accept_shared(&v))
        else {
            return Received::Refused("the sealed profile is not a profile".into());
        };
        if let Some(member) = self.members.get_mut(experiment).and_then(|m| m.get_mut(&from)) {
            member["profile"] = accepted.clone();
            member["profile_at"] = signed.get("at").cloned().unwrap_or(Value::Null);
        }
        self.save_members();
        Received::Kept(json!({ "from": from, "profile": accepted }))
    }

    /// `members.list`: everyone seen on this experiment, with whatever they
    /// shared with us.
    pub fn list_members(&self, experiment: &str, creator: Option<&str>) -> Value {
        let mut out: Vec<Value> = self
            .members
            .get(experiment)
            .map(|ms| {
                ms.iter()
                    .map(|(key, m)| {
                        json!({
                            "key": key,
                            "short": key.chars().take(8).collect::<String>(),
                            "creator": Some(key.as_str()) == creator,
                            "profile": m.get("profile").cloned().unwrap_or(Value::Null),
                            "last_seen": m.get("last_seen").cloned().unwrap_or(Value::Null),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by(|a, b| b["last_seen"].as_str().cmp(&a["last_seen"].as_str()));
        json!({ "experiment": experiment, "members": out })
    }

    // ------------------------------------------------------------ internals

    /// The members we would seal for: known box key, not us, most recently
    /// seen first, at most `MAX_RECIPIENTS`.
    fn recipients(&self, experiment: &str, me: &str) -> Vec<(String, String)> {
        let mut all: Vec<(String, String, String)> = self
            .members
            .get(experiment)
            .map(|ms| {
                ms.iter()
                    .filter(|(k, _)| k.as_str() != me)
                    .filter_map(|(k, m)| {
                        let b = m.get("box").and_then(Value::as_str)?;
                        let seen = m.get("last_seen").and_then(Value::as_str).unwrap_or("").to_string();
                        Some((k.clone(), b.to_string(), seen))
                    })
                    .collect()
            })
            .unwrap_or_default();
        all.sort_by(|a, b| b.2.cmp(&a.2));
        all.truncate(MAX_RECIPIENTS);
        all.into_iter().map(|(k, b, _)| (k, b)).collect()
    }

    fn note_member(&mut self, experiment: &str, key: &str, box_key: Option<&str>, now: DateTime<Utc>) {
        let members = self.members.entry(experiment.to_string()).or_default();
        let member = members
            .entry(key.to_string())
            .or_insert_with(|| json!({ "first_seen": now.to_rfc3339() }));
        member["last_seen"] = json!(now.to_rfc3339());
        if let Some(b) = box_key {
            // A changed box key means a reinstalled node under the same
            // identity; the signature already bound it to `key`, so take it.
            member["box"] = json!(b);
        }
        self.save_members();
    }

    fn save_members(&self) {
        let Some(dir) = &self.dir else {
            return;
        };
        let map: Map<String, Value> = self
            .members
            .iter()
            .map(|(e, ms)| (e.clone(), Value::Object(ms.clone().into_iter().collect())))
            .collect();
        if let Ok(bytes) = serde_json::to_vec(&Value::Object(map)) {
            if let Err(e) = profile::write_private(&dir.join(MEMBERS_FILE), &bytes) {
                eprintln!("collect_core: cannot save members: {e}");
            }
        }
    }

    fn history_path(&self, experiment: &str) -> Option<PathBuf> {
        // The experiment id is the contract's base32 shape; refuse anything
        // else rather than build a path from it.
        if experiment.len() != 16 || !experiment.chars().all(|c| matches!(c, 'a'..='z' | '2'..='7')) {
            return None;
        }
        Some(self.dir.as_ref()?.join(MESSAGES_DIR).join(format!("{experiment}.jsonl")))
    }

    fn history(&mut self, experiment: &str) -> &mut Vec<Value> {
        if !self.messages.contains_key(experiment) {
            let mut loaded = Vec::new();
            if let Some(path) = self.history_path(experiment) {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    loaded = text
                        .lines()
                        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                        .collect();
                }
            }
            // The file is appended in arrival order; show send order.
            loaded.sort_by(|a: &Value, b: &Value| a["at"].as_str().unwrap_or("").cmp(b["at"].as_str().unwrap_or("")));
            self.messages.insert(experiment.to_string(), loaded);
        }
        self.messages.get_mut(experiment).expect("just inserted")
    }

    fn append_history(&self, experiment: &str, m: &Value) {
        let Some(path) = self.history_path(experiment) else {
            return;
        };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
            writeln!(f, "{m}")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            }
            Ok(())
        };
        if let Err(e) = write() {
            eprintln!("collect_core: cannot keep message history: {e}");
        }
    }

    fn rewrite_history(&self, experiment: &str) {
        let (Some(path), Some(msgs)) = (self.history_path(experiment), self.messages.get(experiment)) else {
            return;
        };
        let body: String = msgs.iter().map(|m| format!("{m}\n")).collect();
        if let Err(e) = profile::write_private(&path, body.as_bytes()) {
            eprintln!("collect_core: cannot rewrite message history: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{identity::Identity, schema};

    const EXP: &str = "q7k2m3x4w3b5n6tz";

    struct Node {
        dir: PathBuf,
        id: Identity,
        social: Social,
    }

    impl Node {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("collect-social-{}", uuid::Uuid::now_v7()));
            let id = Identity::load_or_create(&dir).unwrap();
            let mut social = Social::default();
            social.attach(&dir).unwrap();
            Node { dir, id, social }
        }
        fn me(&self) -> String {
            self.id.public_hex()
        }
        fn say(&mut self, text: &str, now: DateTime<Utc>) -> Value {
            let m = self.social.compose(EXP, &self.me(), text, None, now).unwrap();
            let signed = self.id.sign(&m).unwrap();
            assert_eq!(schema::errors_as(&signed, schema::Kind::MessageAnnouncement), Vec::<String>::new());
            let me = self.me();
            self.social.receive_message(EXP, &signed, &me, now);
            signed
        }
        fn introduce(&mut self, now: DateTime<Utc>) -> Value {
            let me = self.me();
            let p = self.social.compose_profile(EXP, &me, now).unwrap();
            let signed = self.id.sign(&p).unwrap();
            assert_eq!(schema::errors_as(&signed, schema::Kind::ProfileAnnouncement), Vec::<String>::new());
            self.social.mark_shared(EXP, &me, now);
            signed
        }
        fn hear(&mut self, signed: &Value, now: DateTime<Utc>) -> Received {
            crate::sign::verify(signed).unwrap();
            let me = self.me();
            match signed["kind"].as_str() {
                Some("message") => self.social.receive_message(EXP, signed, &me, now),
                _ => self.social.receive_profile(EXP, signed, &me, now),
            }
        }
    }

    impl Drop for Node {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn t(s: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-29T12:00:00Z").unwrap().with_timezone(&Utc) + Duration::seconds(s)
    }

    #[test]
    fn a_message_reaches_another_member_and_is_kept_once() {
        let mut alice = Node::new();
        let mut bob = Node::new();
        let m = alice.say("Found three kites by the lake", t(0));
        assert!(matches!(bob.hear(&m, t(1)), Received::Kept(_)));
        assert_eq!(bob.hear(&m, t(2)), Received::Duplicate, "a replay was kept twice");
        let list = bob.social.list_messages(EXP, &bob.me());
        assert_eq!(list["messages"].as_array().unwrap().len(), 1);
        assert_eq!(list["messages"][0]["text"], json!("Found three kites by the lake"));
        assert_eq!(list["messages"][0]["mine"], json!(false));
    }

    #[test]
    fn history_survives_a_restart_and_stays_in_send_order() {
        let mut alice = Node::new();
        let mut bob = Node::new();
        let late = alice.say("second", t(10));
        let early = alice.say("first", t(5));
        bob.hear(&late, t(11));
        bob.hear(&early, t(12));
        let mut again = Social::default();
        again.attach(&bob.dir).unwrap();
        let list = again.list_messages(EXP, &bob.me());
        let texts: Vec<&str> = list["messages"].as_array().unwrap().iter().map(|m| m["text"].as_str().unwrap()).collect();
        assert_eq!(texts, ["first", "second"]);
    }

    #[test]
    fn empty_and_overlong_messages_are_refused() {
        let alice = Node::new();
        assert!(alice.social.compose(EXP, &alice.me(), "   ", None, t(0)).is_err());
        let long = "x".repeat(MAX_TEXT + 1);
        assert!(alice.social.compose(EXP, &alice.me(), &long, None, t(0)).is_err());
    }

    #[test]
    fn a_shared_field_reaches_only_its_recipient() {
        let mut alice = Node::new();
        let mut bob = Node::new();
        let mut carol = Node::new();
        alice.social.save_profile(&json!({ "fields": {
            "alias": { "value": "Alice", "share": true },
            "phone": { "value": "+91 98450 12345", "share": true },
            "email": { "value": "alice@example.org", "share": false },
        }})).unwrap();
        // Bob speaks first, so Alice learns his box key.
        let hello = bob.say("hello", t(0));
        alice.hear(&hello, t(1));
        let card = alice.introduce(t(2));
        assert!(!card.to_string().contains("98450"), "a shared field went out in the clear");
        assert!(!card.to_string().contains("alice@example.org"));

        match bob.hear(&card, t(3)) {
            Received::Kept(v) => {
                assert_eq!(v["profile"], json!({ "alias": "Alice", "phone": "+91 98450 12345" }));
            }
            other => panic!("bob got {other:?}"),
        }
        // Carol was not a member when Alice sealed, so nothing is hers.
        assert_eq!(carol.hear(&card, t(4)), Received::NothingForUs);
        // Bob's view of Alice's messages now carries her name.
        let m = alice.say("hi bob", t(5));
        bob.hear(&m, t(6));
        let list = bob.social.list_messages(EXP, &bob.me());
        assert_eq!(list["messages"][1]["name"], json!("Alice"));
    }

    #[test]
    fn a_new_member_triggers_a_reshare_after_the_gap() {
        let mut alice = Node::new();
        let mut bob = Node::new();
        alice.social.save_profile(&json!({ "fields": { "alias": { "value": "Alice", "share": true } } })).unwrap();
        let me = alice.me();
        assert!(alice.social.needs_share(EXP, &me, t(0)), "never introduced");
        alice.introduce(t(0));
        assert!(!alice.social.needs_share(EXP, &me, t(1)));
        let hello = bob.say("hi", t(2));
        alice.hear(&hello, t(3));
        assert!(!alice.social.needs_share(EXP, &me, t(30)), "re-shared inside the gap");
        assert!(alice.social.needs_share(EXP, &me, t(RESHARE_GAP_SECS + 1)));
    }

    #[test]
    fn changing_ones_profile_reshares_at_once() {
        let mut alice = Node::new();
        let me = alice.me();
        alice.introduce(t(0));
        assert!(!alice.social.needs_share(EXP, &me, t(1)));
        alice.social.save_profile(&json!({ "fields": { "age": { "value": 40, "share": true } } })).unwrap();
        assert!(alice.social.needs_share(EXP, &me, t(2)));
    }

    #[test]
    fn a_member_with_nothing_shared_only_introduces_once() {
        let mut alice = Node::new();
        let mut bob = Node::new();
        let me = alice.me();
        let card = alice.introduce(t(0));
        assert_eq!(card["sealed"], json!([]));
        let hello = bob.say("hi", t(1));
        alice.hear(&hello, t(2));
        assert!(!alice.social.needs_share(EXP, &me, t(500)));
        // And Bob learnt Alice's box key from the introduction.
        assert_eq!(bob.hear(&card, t(3)), Received::NothingForUs);
        assert_eq!(bob.social.recipients(EXP, &bob.me()).len(), 1);
    }

    #[test]
    fn a_received_profile_is_rechecked_and_persisted() {
        let mut alice = Node::new();
        let mut bob = Node::new();
        let hello = bob.say("hello", t(0));
        alice.hear(&hello, t(1));
        alice.social.save_profile(&json!({ "fields": { "alias": { "value": "Alice", "share": true } } })).unwrap();
        let card = alice.introduce(t(2));
        bob.hear(&card, t(3));
        let mut again = Social::default();
        again.attach(&bob.dir).unwrap();
        let members = again.list_members(EXP, None);
        assert_eq!(members["members"][0]["profile"], json!({ "alias": "Alice" }));
    }

    #[test]
    fn history_never_names_a_path_outside_the_directory() {
        let alice = Node::new();
        assert_eq!(alice.social.history_path("../../etc/passwd"), None);
        assert!(alice.social.history_path(EXP).is_some());
    }
}
