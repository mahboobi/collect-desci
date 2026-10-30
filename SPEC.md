# Collect — specification (v1)

A small, standalone Logos Basecamp module. A **creator** defines an experiment (title, description, start and end date) with one or more **protocols**, which are custom data collection forms. **Submitters** open the experiment link in Basecamp, read the instructions and fill in a form. Every experiment, protocol and submission is a signed JSON document stored **publicly and unencrypted on Logos Storage**. Anyone with the link can read everything; only Basecamp users can submit.

Prepared 28 October 2026. Design: `design/canvas/` (7 artboards). Contract: `schemas/collect.schema.json`.

---

## 1. Users

| User | Who | Can |
| --- | --- | --- |
| Creator | The Basecamp identity that created the experiment | Create and edit the experiment and its protocols; open or close protocols; see results; hide a submitter; export |
| Submitter | Any Basecamp user with the link | Read the experiment and all submissions; fill in open protocols while the experiment is open; save drafts; see their own submissions and whether the creator received them |

There are no other roles in v1. The same person can be a creator of one experiment and a submitter to another. A reader who is not submitting is just a submitter who has not submitted.

---

## 2. What is stored, and where

Everything is a JSON document on **Logos Storage**, addressed by CID. Documents are signed with the author's Ed25519 key over their canonical JSON (RFC 8785) without the `sig` field.

| Document (`schema`) | Author | Contents | Mutable? |
| --- | --- | --- | --- |
| `collect/experiment@1` | Creator | id, version, title, description, starts_at, ends_at, timezone, protocol list (id, version, CID, status), `prev` (previous manifest CID), optional `thumbnail` (a `media` entry: 960x540 JPEG cover on Storage, metadata removed; added 30 October 2026) | No. Each edit uploads a new version that points to the previous one. |
| `collect/protocol@1` | Creator | id, version, name, instructions, fields, options | No. Edits create a new version; submissions name the version they used. |
| `collect/submission@1` | Submitter | experiment, protocol id and version, submitter key and optional name, submitted_at, values; media as CIDs | No |
| `collect/index@1` | Creator | One page (up to 1,000 entries) listing received submissions, hidden keys, and `prev` page CID | No. New pages link back to earlier ones. |
| Media files | Submitter | Photos, with all metadata removed | No |

**Messaging.** Storage has no network-wide "list" and no "notify" operation, so documents are announced on one **Logos Delivery** content topic per experiment: `/collect/1/exp-<experimentId>/json`. (Verified: `manifests()` does list what the *local* node holds — see `docs/api-verification.md` Q2.) Messages (schemas in `$defs/announcement`):

| Kind | From | When | Purpose |
| --- | --- | --- | --- |
| `head` | Creator node | On every change and every 10 minutes | Latest manifest CID and latest index CID |
| `submission` | Submitter node | On submit, then every 10 minutes until a receipt arrives (up to 7 days) | Tells the creator a new submission CID exists |
| `receipt` | Creator node | After accepting a submission | Lets the submitter show "Received by creator" |

Delivery Store keeps messages only temporarily, which is why submitters re-announce and the creator publishes the index to Storage. **The index on Storage is the durable list; Delivery is only the doorbell.**

**Verified against the real modules** (28 October 2026, `docs/api-verification.md`). Three things in this section were wrong and one is a blocker:

- There is no pin operation. "The creator's node pins every document" means: the bytes are in its local storage, it never calls `remove()`, and `advertise` stays on. Nothing reserves space or guarantees retention.
- `remove()` exists and deletes from **local** storage, so "nothing can be deleted" is wrong as written. It does not make anything retractable: the DHT record is not withdrawn and other nodes keep what they fetched. A published exact coordinate still cannot be recalled, which is what `rounded_1km` and hide-rather-than-delete depend on.
- Announcements carry a ~150 KiB ceiling (store sync refuses more, whatever `--max-msg-size` says). Collect's announcements are far smaller, so this only confirms that documents go to Storage and only CIDs go over Delivery.
- **Blocker: `logos.test` requires a funded RLN membership before a node starts.** §1's "any Basecamp user with the link can submit" and §3.2's submitter-publishes-an-announcement do not hold there, and obtaining a membership needs testnet tokens, which `CLAUDE.md` rules out for v1. Open question Q9 below.

**The link.** `logos-collect://exp/<experimentId>?m=<manifestCid>`. `experimentId` = first 10 bytes of SHA-256(creator key ‖ random nonce), base32 lower-case. On open, the module fetches the manifest from `m`, subscribes to the topic, and switches to a newer manifest when a valid creator-signed `head` arrives (same experiment id, higher version, same creator key).

**Reading without being the creator.** Fetch the newest index page from `head` (or from the manifest's `index` field), walk `prev` pages, and fetch the listed submission CIDs. This is how "anyone with the link can read" works. It needs a Logos Storage node, which Basecamp provides. Reading from a plain web browser needs an HTTP gateway (open question Q3).

---

## 3. Flows

### 3.1 Creator: create and publish
1. **C1 Home → New experiment → C2.** Enter title, description, start date, end date and time zone. The end date must not be before the start date (inline error; Save disabled).
2. **Add protocol → C3 builder.** Fill in name, instructions and fields (types in §4), choose whether people may submit more than once and whether drafts are allowed, then **Save and publish**.
3. **Save and publish on C2.** `collect_core` validates, uploads protocols and then the manifest, publishes a `head`, and shows the link with **Copy link**.
4. The creator's node subscribes to the topic, pins every document of its own experiment, and stays the indexer.

### 3.2 Submitter: open and submit
1. Paste the link (C1 "You submit to" tab, or the "Open a link" tab) → **S1**: title, description, dates, creator name and key fingerprint, public-data notice, list of protocols (Open / Closed), "Your submissions".
2. **Fill in form → S2.** The instructions box is at the top; then fields in protocol order; "required"/"optional" labels; Submit is disabled until required fields are valid.
3. **Submit:** `collect_core` validates, processes media (§5), applies location rounding if the protocol says so, signs, uploads media, then the submission JSON, announces it, and shows **S3**: signed ✓, saved on Logos Storage (CID) ✓, announced ✓, received by the creator's node (waiting → done on receipt).
4. Outside the experiment dates, or when the protocol is closed, S1 shows the form as not accepting submissions and S2 cannot be opened.

### 3.3 Creator: receive, view and export
For each `submission` announcement, the creator node:
1. fetches the CID. Verified: `downloadToUrl` **is** asynchronous and completes on `storageDownloadDone`, whose payload carries no CID — only `success`, `sessionId` and `error` — so the fetch is keyed on the session id, which the contract says equals the CID (`docs/api-verification.md` round 4);
2. checks the signature, schema, protocol id and version, field validation (§4), media CIDs fetchable, sizes;
3. checks the time window: the receive time is between `starts_at` and `ends_at`. If drafts are allowed, it also accepts a receive time up to 7 days after `ends_at` when `submitted_at` is inside the window;
4. checks the rate limit (200 per submitter key per day, configurable) and hidden keys;
5. keeps the submission and its media, appends the entry to the current index page, publishes the changed page, re-signs the manifest to point at it, and sends a `receipt`. **Not batched:** each accepted submission publishes immediately. Batching would hold a receipt back for up to a minute to save a CID, and the receipt is what the submitter is waiting on. There is also **no pin** — keeping a document means the bytes are in local storage, `remove()` is never called, and `advertise` stays on (§2).

Invalid submissions are ignored and logged locally; they are never added to the index.

**C4 Results:** summary cards (submissions, submitters, storage size, waiting to be picked up — **not** copies: no API reports a replication count, so the card shows only copies we know of, or drops the number), one tab per protocol, a table whose columns follow the protocol's fields, search, View (full submission with photos), Copy link, Export (CSV per protocol + media folder + JSON of all documents), submissions per day (last 14 days), most active submitters, hidden submitters with Unhide.

**Hide a submitter** (from View): their key goes into `hidden_keys` on the next index page. Their files stay on Storage (nothing can be deleted), but they are dropped from the index, results and exports.

---

## 4. Protocol fields

| Type | Settings | Stored value |
| --- | --- | --- |
| `text` | max_length (default 200) | string |
| `longtext` | max_length (default 5000) | string |
| `number` | min, max, integer, unit | number |
| `choice` | options (2–50, each `{value, label}`) | option value |
| `multi` | options, min, max | array of option values |
| `datetime` | fill_from (`photo`, `now`), no_future | RFC 3339 with offset |
| `photo` | min, max (≤ 10), max_mb_each (default 25), fill_from_exif | `[{cid, sha256, mime, bytes, width, height}]` |
| `location` | methods (`photo`, `map`, `manual`), max_accuracy_m, store (`exact` or `rounded_1km`) | `{lat, lon, accuracy_m, method}` |
| `yesno` | — | boolean |

Every field has `key` (stable, `^[a-z][a-z0-9_]{0,39}$`), `label`, optional `help`, and `required`. Keys and option values may not change once a version is published; labels may. A new protocol version is created on publish when anything changed.

`rounded_1km` snaps to the centre of a 0.01° × 0.01° cell **on the submitter's computer before storage**. The exact point is never stored, because all data is public.

---

## 5. Media rules

1. Read EXIF/XMP: GPS position, accuracy if present, `DateTimeOriginal` with offset, and orientation.
2. Apply the orientation to the pixels, then **remove all metadata** (EXIF, XMP, IPTC, maker notes). Upload only the cleaned file. Add a test that cleaned files contain no GPS tags.
3. Use the values read in step 1 to prefill `location` (method `photo`) and `datetime` when the protocol allows it.
4. Reject files over `max_mb_each`. Accept JPEG, PNG and WebP; accept HEIC only if decoding is available.

---

## 6. `collect_core` API

The table at the end of this section is the **original proposal**. What is
built differs, and where it differs the built names win. The shipped surface,
all through one `call(method, request)` entry point plus the typed trait
methods beside it:

| Area | Built |
| --- | --- |
| Session | `session.capabilities` → `{signing, publishing, store, …}` |
| Identity | `identity.me` → `{key, signing}` |
| Experiments | `experiments.list`, `experiments.get {experimentId}`, `experiments.create {title, description, startsAt, endsAt, timezone, creatorName}`, `experiments.update {experimentId, …}`, `experiments.publish {experimentId}` (retry an upload), `experiments.setThumbnail {experimentId, path}` (path or `file://` URL; needs Storage; makes a new version), `experiments.clearThumbnail {experimentId}`, `experiments.open {link}`, `experiments.parseLink`. `list` cards and `get` carry `thumbnail`: a data URI the UI can show, or null when there is none or this node does not hold it yet (the drain fetches missing ones, two per run, and checks each against the manifest's sha256) |
| Protocols | `protocols.get`, `protocols.saveDraft`, `protocols.validateDraft` |
| Submissions | `submissions.validate`, `submissions.prepareValues`, `submissions.processMedia` |
| Index | `index.accept`, `index.page` |
| Results | `results.list {experimentId, protocolId}`, `results.export {experimentId, protocolId, format, includeMedia}` → `{path, rows, bytes, media}` |
| Moderation | `submitters.hide {experimentId, submitter}`, `submitters.show {…}` |
| Delivery | `delivery.status` (includes `connection: {level: online \| degraded \| offline, reason}` for the dashboard dot, from Delivery's `getConnectionStatus`, Storage presence and RLN readiness), `delivery.drain`, `delivery.publishHead {experimentId}`, `delivery.announceSubmission {experimentId, cid, protocolId, version}`, `delivery.deliver {topic, messageHash, payload, source}` |
| Geo | `geo.round` |

Differences worth naming:

- **`experiments.publish` is a retry, not the publish step.** Creating an
  experiment uploads it; `publish` exists because `storage_timeout` and
  `storage_rejected` are retryable and one bad moment should not leave a
  signed, valid experiment permanently unaddressable.
- **Hiding is `submitters.hide`**, not `results.hideSubmitter`: it rewrites
  the index, which is not a property of a results view.
- **`results.export` returns a path, not a job.** It is fast enough to be
  synchronous for the sizes involved, and the path is what the UI needs —
  the QML side has no filesystem, so core writing the file is the only way a
  creator ends up with one.
- **No `experiments.listMine` / `listJoined`.** One `experiments.list`
  returns cards carrying `mine`, because the dashboard shows both sections
  from one call and two calls could disagree.
- **No `protocols.publish` / `setStatus` / `experiments.forget` yet.**
- **`session.status` is `session.capabilities`**, and it reports what this
  build can do rather than what the network is doing.

### The original proposal

Same conventions as Open Labbook: `logos.callModule("collect_core", method, [jsonString])` returns `{"ok": true, "data": …}` or `{"ok": false, "error": {"code", "message"}}`. Long jobs return `{job}` and report progress through events.

**Verified (Q1), and the JSON-string convention is forced rather than chosen.** In the `rust` template a Rust **trait is the contract** — the builder derives the `.lidl` from it, so there is no `.lidl` to write and no `build.rs`. Two constraints follow:

- The trait name is derived from the module name: `collect_core` -> `CollectCoreModule`, and it must be `Send + 'static`.
- **Only `String`, `i64`, `u64`, `f64`, `bool`, `Vec<u8>` and `Option<T>` cross the ABI.** No structs or maps, so a protocol or submission has to travel as a JSON string. The `{ok, data}` envelope is still our own convention, not the platform's.

Events are a second trait (`CollectCoreModuleEvents`); each method becomes a free `emit_<name>(...)` taking strings by reference. Dependencies named in `metadata.json` arrive as typed clients on `modules()` — `modules().storage_module...`, `modules().delivery_module...` — with `_async` twins, and a dependency's events come back as an iterable `EventSubscription` that can move into a listener thread.

**Every call already has a timeout**: the protocol default is 20 s, with a `_with_timeout(..., Duration)` twin per method. So §3.3's 10-minute give-up is `..._with_timeout(..., Duration::from_secs(600))` — and note the 20 s default would otherwise cut a Storage fetch off far too early.

Durable state goes in `instance_persistence_path` from `RustModuleContext`, read through `context()` and available from `on_context_ready`.

**For the UI, §6's assumption does not hold.** A `ui-qml-backend` module reaches its *own* backend through a typed QTRO replica, not `callModule`: `.rep` `PROP()`s auto-sync as QML properties and `SLOT()`s are called directly, with the pending reply delivered by `logos.watch(call, onValue, onError)`, plus `logos.isViewModuleReady(name)` / `onViewModuleReadyChanged`. `callModule` / `callModuleAsync` are for reaching a *different* module, which is how the UI side would reach `collect_core`. See Q12.

| Area | Methods |
| --- | --- |
| Session | `session.status` → node, storage, delivery state |
| Identity | `identity.me` → `{key, fingerprint, name}`; `identity.setName {name}` |
| Experiments | `experiments.listMine`, `experiments.listJoined`, `experiments.create {title, description, startsAt, endsAt, timezone}`, `experiments.update {experimentId, …}`, `experiments.publish {experimentId}` → `{manifestCid, link}`, `experiments.open {link}` → view model for S1, `experiments.forget {experimentId}` |
| Protocols | `protocols.get {protocolId, version?}`, `protocols.saveDraft {experimentId, protocolId?, name, instructions, fields, allowMultiple, allowDrafts}`, `protocols.validateDraft` → `{errors[], changes[]}`, `protocols.publish {protocolId}` → `{version, cid}`, `protocols.setStatus {protocolId, status: open or closed}` |
| Submissions | `submissions.newDraft {experimentId, protocolId}` → `{draftId, form, prefills}`, `submissions.addMedia {draftId, fieldKey, path}` → job, `submissions.removeMedia`, `submissions.saveDraft {draftId, values}`, `submissions.validate {draftId}` → `{missing[], errors{}}`, `submissions.submit {draftId}` → `{cid}`, `submissions.mine {experimentId}` → rows with state `draft`, `announced` or `received` |
| Results | `results.summary {experimentId}`, `results.list {experimentId, protocolId, query?, limit, before?}`, `results.get {submissionCid}`, `results.hideSubmitter {experimentId, key}`, `results.unhideSubmitter`, `results.export {experimentId, protocolId?, format: csv or json, includeMedia}` → job → output path |

**Events:** `syncStatusChanged`, `experimentUpdated {experimentId, manifestCid}`, `submissionReceived {experimentId, protocolId, cid}` (creator), `receiptReceived {cid}` (submitter), `mediaProcessed {draftId, fieldKey, item, exif}`, `jobProgress {job, state, detail}`.

**Fixtures mode** (`COLLECT_FIXTURES=1`): read methods return `examples/*.json`, writes succeed in memory, receipts arrive after 3 s.

---

## 7. Screens (design files in `design/canvas/`)

| Id | File | Who | Purpose |
| --- | --- | --- | --- |
| C1 | `Main.dc.html` | Both | Home: "Created by you" cards (Open, Draft, Ended) and "You submit to" list with "Open an experiment from a link" |
| C2 | `Experiment.dc.html` | Creator | Details (title, description, dates with validation, time zone), protocol list (Edit, Close/Reopen, + Add protocol), Share (link, Copy), "All data is public" warning, Storage |
| C3 | `Builder.dc.html` | Creator | Protocol builder: name, instructions, fields list (select, reorder, remove, add from 9 types), field settings, live form preview, submission options |
| C4 | `Results.dc.html` | Creator | Results: summary, protocol tabs, table, export, per-day chart, active and hidden submitters |
| S1 | `Join.dc.html` | Submitter | Experiment page from a link: description, dates, public notice, forms, your submissions |
| S2 | `Form.dc.html` | Submitter | The generated form with instructions at the top |
| S3 | `Submitted.dc.html` | Submitter | Confirmation with storage and delivery steps |

The UI is a **form renderer** driven by the protocol JSON. The wildlife form in the design is only an example; never hard-code it. Visual system: the Open Labbook "lab ledger" tokens (`design/tokens.json`, `design/Theme.qml`): IBM Plex Sans and Mono, green accent `#0B6E4F`, violet `#4B3FB0` for the selected/current item, amber `#8A4B00` for warnings, 44 px targets.

---

## 8. Acceptance tests (scaffold profiles `creator`, `alice`, `bob`, `reader`)

`basecamp setup` seeds only `alice` and `bob`. Declare the other two under `[basecamp.profiles.<name>]` in `scaffold.toml`; unknown profiles are seeded on first launch.

1. `creator` creates an experiment with two protocols and publishes; `alice` opens the link and sees both forms within 10 s.
2. `alice` submits with a JPEG that has GPS EXIF: location and time prefill; the uploaded photo has no metadata (automated check); `creator` sees the row within 30 s; `alice` sees "Received by creator".
3. `creator` offline: `bob` submits; `creator` starts 15 min later; the submission appears without `bob` doing anything (re-announce works).
4. Before the start date and after the end date the form cannot be opened; a hand-crafted submission with a receive time outside the window is not indexed.
5. A closed protocol is not offered; its announcements are ignored.
6. A protocol with `rounded_1km` stores only rounded coordinates (inspect the JSON on Storage).
7. `reader` (never submitted, not the creator) opens the link and lists every submission from the index pages.
8. Hiding `bob` removes his rows from results and export, and from the next index page.
9. Editing a protocol publishes version 2; earlier rows show version 1; the CSV has a `protocol_version` column.
10. All documents written during tests validate against `schemas/collect.schema.json`.

---

## 10. Science profile and project discussion (added 29 October 2026)

Two additions requested after the first live test. **Neither ever writes to Logos Storage.**

### 10.1 Profile

Opened from the **Profile** button at the top right of every screen. Fields, all optional: name or alias, affiliation, location (a city or region, not GPS), sex (female, male, intersex, other, prefer not to say), age (1 to 130), email, phone, wallet address (public address only), and a photo.

- Kept in `profile.json` in the module's persistence directory, readable by the owner only. A file that cannot be read is reported and never overwritten.
- Each field has its own **Share with project members** switch. Off by default.
- The photo is passed through the same metadata removal as submission photos, cropped square, and kept as a 128 px JPEG. The shared copy is 64 px. The original file is never kept.
- Validation checks shape only. This is self-description, not verification.

### 10.2 Sharing a profile

A shared field is sealed **separately for each member** and sent as a `profile` announcement on the experiment's Delivery topic (contract: `announcement` kind `profile`).

- Each installation has an X25519 "box" key beside its Ed25519 signing key. The box key travels inside signed announcements, so the signature binds it to the member.
- For each recipient: X25519, then HKDF-SHA256 with an `info` naming the purpose, both box keys and both identities, then ChaCha20-Poly1305 with a random nonce. The associated data is the experiment id plus both identities, so an entry cannot be moved to another experiment or re-addressed.
- **Members** are the keys whose signed `message` or `profile` arrived on the topic. A node introduces itself with a `profile` announcement as soon as it can, with an empty `sealed` list if it shares nothing. It re-sends when its own shared fields change (at once) or a new member appears (at most once a minute). At most 32 recipients per announcement, the most recently seen first.
- What stays visible to anyone on the topic: who shares with whom, when, and roughly how much.
- A received profile is re-validated, unknown fields are dropped, and a photo must be a small JPEG data URI.

### 10.3 Discussion

Opened from **Discussion** on the experiment screen.

- A `message` announcement (contract: kind `message`): UUIDv7 id, 1 to 2000 characters of plain text, the sender's box key, signed.
- **Readable by anyone with the experiment's link.** The screen says so. Names and photos appear only for members who shared them with this node.
- Kept per experiment in `messages/<experimentId>.jsonl`, at most 2000 per experiment, sorted by the sender's timestamp, and deduplicated by sender and id (Store catch-up re-delivers under new message hashes). A message is kept only once Delivery accepted it.
- Sending needs RLN, like every announcement (Q9).

### 10.4 API

`profile.get`, `profile.save`, `profile.setPhoto` (`{ path, share }`, read by `collect_core` because the UI sandbox cannot read files), `profile.clearPhoto`, `messages.list`, `messages.send` (`{ experimentId, text, replyTo? }`), `members.list`. `delivery.drain` also sends due profile announcements and reports them as `profiles_shared`.

---

## 9. Open questions

Raised by the profile and discussion work, 29 October 2026:

| # | Question | State |
| --- | --- | --- |
| Q15 | Does the Basecamp UI sandbox allow `QtQuick.Dialogs`? | **Open — check in the app.** The photo picker loads through a `Loader`. If the import is refused, the profile screen falls back to a typed file path. **Answered 30 October 2026 (Basecamp 0.3.2):** the QtQuick FileDialog does open, but it is parented to the view's offscreen window, so the person is left outside Basecamp. On macOS the backend now runs the chooser as `osascript` and reactivates Basecamp (`pickImage` / `imagePicked`). The sandbox also refuses `data:` image URLs, so the backend writes every image in a reply into `plugins/collect_ui/cache/` and passes a `file://` URL. |
| Q16 | What RLN rate limit applies to messages on `logos.test`? | **Open.** Every message and profile announcement uses the sender's RLN quota. If the quota is one message per 600 s epoch, discussion is unusable on testnet. |
| Q17 | Projects with more than 32 members | **Open.** Only the 32 most recently seen members get a sealed profile. A larger project would need several announcements, or a group key. |

Two raised by wiring Storage and Delivery, 29 October 2026:

| # | Question | State |
| --- | --- | --- |
| Q13 | **New.** Where should an export go? | **Open, and currently answered by default.** It is written under the module's `instance_persistence_path`, which is the only directory we are promised is writable — but it is somewhere no one would look, so the UI shows the full path. Writing to `~/Downloads` would be findable and is also a module writing outside its own space without being asked. Needs a decision, possibly a file dialog the host does not currently offer. |
| Q14 | **New.** Should the creator's node fetch a submission it has not been announced? | **Open.** The index can name a CID this node never fetched (`not_fetched` on the C4 cards). Nothing currently goes and gets them, so a creator who was offline during an announcement sees the count but not the rows until the submitter re-announces. A catch-up pass over the index would fix it. |


Answered 28 October 2026 against the published APIs and `lgs basecamp docs`. Evidence and quotes: `docs/api-verification.md`. Nothing below was verified by running a node — Nix was not installed — so items marked **needs a node** are still open.

| # | Question | Status |
| --- | --- | --- |
| Q1 | How do the 0.3.0 `rust` and `ui-qml-backend` templates expose methods, return values and events? | **Answered** (both read from the templates). Rust: a trait is the contract, name derived from the module name, `Send + 'static`, and only scalar/`String`/`Vec<u8>`/`Option<T>` cross the ABI. Events are a second trait emitting `emit_<name>`. Dependencies are typed clients on `modules()` with `_async` and `_with_timeout` twins (20 s default). QML: `ui-qml-backend` uses a `.rep` replica with `logos.watch`, not `callModule`. |
| Q2 | Storage API: pin or keep a CID, fetch timeouts, and can we count copies held by other nodes? | **Answered.** No pin — `setAdvertise()` only controls DHT announcement, and keeping a document means not calling `remove()`. Timeouts come from the call layer (`_with_timeout`), not from Storage. **No copy count exists**, so the C4 card cannot show one. Bonus: `manifests()` lists local content, which §2 had ruled out. |
| Q3 | Is there a public HTTP gateway for Logos Storage, so the link can be read in a web browser? | **Leaning no.** The Storage API is a module API over local IPC with no HTTP read path. Default stands: reading needs Basecamp or a Logos node. |
| Q4 | Delivery Store retention, message size limit, and whether RLN membership is needed to publish on testnet | **Answered, and it is a blocker — see Q9.** RLN **is** required on `logos.test`: a node needs the RLN modules and a funded membership before `start` passes, with 600 s epochs. Effective message size ceiling ~150 KiB. Retention is operator-configured (Store needs PostgreSQL and a policy such as `size:1GB`), so the 7-day re-announce window is our policy, not a guarantee. Better than assumed: `messageReceived` carries `source: "live" | "history"` and Store catch-up is on by default, and `storeQuery` can ask for a time range directly. |
| Q5 | Map tiles in QML (network from a core module? offline tiles?) | **Unchanged.** Nothing lifts the UI sandbox. Placeholder map plus typed coordinates; photo GPS works without a map. |
| Q6 | Optional always-on **mirror** node (headless `collect_core` that pins and re-publishes the index when the creator is offline) | Not in v1; recommend for any experiment that runs for weeks. Q8 makes this more attractive, since a mirror would also hold durable state. |
| Q7 | More than one creator per experiment | Not in v1. |
| Q8 | **New, now mostly answered.** Where does `collect_core` keep state that must survive a relaunch? | `RustModuleContext::instance_persistence_path` — "a writable, per-instance directory, the place to keep a module's on-disk state", read via `context()` from `on_context_ready`. The scrub in `lgs basecamp docs` is `lgs basecamp launch`'s deliberate clean-slate contract, not Basecamp's, so in normal use this persists. What remains is a **testing** problem: §8 test 3 restarts the creator, so hand-launch with `--user-dir` outside the scaffold profile tree. Until 30 October 2026 the code never wrote there, so every relaunch emptied the dashboard; experiments, forms, submissions, index pages, published CIDs, receipts, heads and seen message hashes are now saved to `state.json` in that directory after every call and re-validated on load. |
| Q9 | **Answered: no.** Does a Basecamp 0.3.0 profile come with a usable RLN membership? | **No.** With every module in place — `liblogos_rln_module` and `liblogos_lez_rln_module`, neither shipped with basecamp — RLN reaches `rln Ready` and the lez wallet initialises. Delivery then says: `No usable RLN membership; proof validation is unaffected, sends fail until it is active (status=Unknown)`. The wallet also reports `initial sync incomplete: sync to 68999 failed (code 13)`. So announcements cannot go out from a stock profile, and acquiring a membership needs testnet funds, which `CLAUDE.md` rules out for v1. Everything up to the send works regardless: a submission is signed and on Storage with a real CID before it is announced, and the creator's side indexes from Storage. **Caution:** `rlnState()` reporting `Ready` describes the subsystem, not the membership — a send gated only on that state would be attempted and fail asynchronously via `messageError`, which Collect does not currently watch. Closing that is the next Delivery task. |
| Q10 | Spam: is the per-key rate limit enough, given that anyone can make new keys? | Rate limit plus hide; revisit if abused. (Was Q8.) RLN, if it does apply to submitters, is itself a per-identity rate limit and partly answers this. |
| Q12 | **Decided: stay with `ui-qml-backend`.** | The pass-through turned out to be one `SLOT` — `call(method, request)` — not one per method, so the layer it would delete is about ten lines. Against deleting it: the auto-syncing `PROP`s (`capabilities`, `coreReady`, `documents`) are what make the dashboard populate without a call, and that is also what masked the QtRO async bug for a while. Keeping the pin. |
| Q11 | **New.** HEIC decoding. | **Open.** The contract allows `image/heic`, but decoding needs a system codec. `collect_core` recognises HEIC and refuses it with its own reason rather than calling it an unknown format. Either find a decoder or drop `image/heic` from the contract. |
