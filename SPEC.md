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
| `collect/experiment@1` | Creator | id, version, title, description, starts_at, ends_at, timezone, protocol list (id, version, CID, status), `prev` (previous manifest CID) | No. Each edit uploads a new version that points to the previous one. |
| `collect/protocol@1` | Creator | id, version, name, instructions, fields, options | No. Edits create a new version; submissions name the version they used. |
| `collect/submission@1` | Submitter | experiment, protocol id and version, submitter key and optional name, submitted_at, values; media as CIDs | No |
| `collect/index@1` | Creator | One page (up to 1,000 entries) listing received submissions, hidden keys, and `prev` page CID | No. New pages link back to earlier ones. |
| Media files | Submitter | Photos, with all metadata removed | No |

**Messaging.** Storage has no "list" or "notify" operation, so documents are announced on one **Logos Delivery** content topic per experiment: `/collect/1/exp-<experimentId>/json`. Messages (schemas in `$defs/announcement`):

| Kind | From | When | Purpose |
| --- | --- | --- | --- |
| `head` | Creator node | On every change and every 10 minutes | Latest manifest CID and latest index CID |
| `submission` | Submitter node | On submit, then every 10 minutes until a receipt arrives (up to 7 days) | Tells the creator a new submission CID exists |
| `receipt` | Creator node | After accepting a submission | Lets the submitter show "Received by creator" |

Delivery Store keeps messages only temporarily, which is why submitters re-announce and the creator publishes the index to Storage. **The index on Storage is the durable list; Delivery is only the doorbell.**

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
1. fetches the CID (gives up after 10 min, retries on the next announcement);
2. checks the signature, schema, protocol id and version, field validation (§4), media CIDs fetchable, sizes;
3. checks the time window: the receive time is between `starts_at` and `ends_at`. If drafts are allowed, it also accepts a receive time up to 7 days after `ends_at` when `submitted_at` is inside the window;
4. checks the rate limit (200 per submitter key per day, configurable) and hidden keys;
5. pins the submission and its media, appends the entry to the current index page, publishes a new page CID on change (batched every 60 s), and sends a `receipt`.

Invalid submissions are ignored and logged locally; they are never added to the index.

**C4 Results:** summary cards (submissions, submitters, storage size and copies, waiting to be picked up), one tab per protocol, a table whose columns follow the protocol's fields, search, View (full submission with photos), Copy link, Export (CSV per protocol + media folder + JSON of all documents), submissions per day (last 14 days), most active submitters, hidden submitters with Unhide.

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

## 6. `collect_core` API (proposed)

Same conventions as Open Labbook: `logos.callModule("collect_core", method, [jsonString])` returns `{"ok": true, "data": …}` or `{"ok": false, "error": {"code", "message"}}`. Long jobs return `{job}` and report progress through events. Verify how the module templates expose methods and events before building (Q1).

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

## 9. Open questions

| # | Question | Default until answered |
| --- | --- | --- |
| Q1 | How do the 0.3.0 `rust` and `ui-qml-backend` templates expose methods, return values and events? | Adapt the API names in §6 to the template |
| Q2 | Storage API: pin or keep a CID, fetch timeouts, and can we count copies held by other nodes? | Count copies we know about: creator, submitter, mirror |
| Q3 | Is there a public HTTP gateway for Logos Storage, so the link can be read in a web browser? | Reading needs Basecamp or a Logos node |
| Q4 | Delivery Store retention, message size limit, and whether RLN membership is needed to publish on testnet | Re-announce every 10 min for up to 7 days |
| Q5 | Map tiles in QML (network from a core module? offline tiles?) | Placeholder map plus typed coordinates; photo GPS works without a map |
| Q6 | Optional always-on **mirror** node (headless `collect_core` that pins and re-publishes the index when the creator is offline) | Not in v1; recommend for any experiment that runs for weeks |
| Q7 | More than one creator per experiment | Not in v1 |
| Q8 | Spam: is the per-key rate limit enough, given that anyone can make new keys? | Rate limit plus hide; revisit if abused |
