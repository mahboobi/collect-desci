# Step 1 — the real Logos APIs, checked against SPEC.md

Verified 28 October 2026 against the published API references, the
`logos-module-builder` README, and `lgs basecamp docs` from the locally
installed `logos-scaffold` 0.4.0 (which prints its own embedded copy of the
compatibility contract, so it is the most authoritative source here).

**Not verified by running anything.** Nix is not installed on this machine, so
no module was built and no profile was launched. Everything below is read from
documentation and the scaffold binary. The items marked **needs a node** cannot
be settled any other way.

---

## Summary of what changed in SPEC.md

| SPEC claim | Reality | Where |
| --- | --- | --- |
| "Storage has no 'list' ... operation" | `manifests()` lists what **this node** holds. No network-wide listing. | §2 |
| "nothing can be deleted" | `remove()` deletes from **local** storage. The network copy and DHT record cannot be withdrawn. | §2, §3.3 |
| "pins every document" | There is no pin operation. Persistence is "stored locally and not removed"; `setAdvertise()` only controls DHT announcement. | §2 |
| C4 shows "storage size and **copies**" | No API returns a replication count. | §3.3 |
| "gives up after 10 min" | No timeout parameter on download; must be a client-side timer. | §3.3 |
| `logos.callModule(..., [jsonString])` returning `{ok, data}` | `callModule` exists, but the `{ok, data}` envelope is an app convention, not the platform's. Typed methods are the platform contract. | §6 |
| Announcements just publish to a topic | **logos.test requires a funded RLN membership before the node starts.** | §2 |
| Re-announce "up to 7 days" | Delivery Store retention is operator-configured, not guaranteed. | §2 |
| (not stated) | Effective message size cap is ~150 KiB. | §2 |
| (not stated) | `module_data/` does not survive a relaunch. | new §10 |

---

## Q1 — how the templates expose methods, return values and events

**Answered for C++, partly open for Rust.**

A core module is "one plain C++ class: its public methods are the module's API,
and `logos-module-builder` generates the Qt plugin around it." Types are the
C-SDK set — `int64_t`, `std::string`, not `int` or `QString`.

From QML there are two calls:

```qml
var result = logos.callModule("storage_module", "save", [noteField.text])

logos.callModuleAsync("my_backend_module", "getStatus", [], function (payload) {
    console.log(JSON.parse(payload).value);
})
```

`LogosQmlBridge` is exposed to QML as `logos` and routes both to backend
modules over IPC; **results are serialized to JSON strings, so QML always sees
a string.** Module-to-module from C++ returns `LogosResult`, with `success()`,
`data()` (a `QVariant`) and `errorMessage()`:

```cpp
LogosResult result = api->callModule("crypto_utils", "hash", {"hello"});
if (result.success()) { std::string h = result.data().toString().toStdString(); }
```

The `ui-qml-backend` template is a different shape from what §6 assumed: the UI
reaches **its own** backend through a `.rep` interface file declaring `PROP()`
and `SLOT()`, bound in QML as

```qml
readonly property var backend: logos.module("notes_app")
```

with readiness via `logos.isViewModuleReady()` and `onViewModuleReadyChanged`.
`callModule` is for reaching a *different* module.

**What this means for §6.** The proposed API survives, but the `{"ok": true,
"data": …}` envelope is our own convention and not something the platform
provides — it has to be implemented inside `collect_core` and parsed in QML.
That is fine and is what §6 already describes; it is just not inherited from
the platform. Long jobs returning `{job}` plus progress events also work, since
events are declared alongside methods in the module class.

**Still open:** the `rust` and `rust-with-external-lib` templates are confirmed
to exist ("Minimal core module written in Rust", "Rust module linking an
external C library"), but **how a Rust module declares its methods, emits
events, and calls other modules is not documented anywhere public.** Reading
the template source needs the flake, so this is blocked on Nix. Until then,
`collect_core` is a plain Rust library with no Logos-facing layer — which is
exactly how it is written today, so nothing is wasted either way.

---

## Q2 — Storage: pin, fetch timeouts, counting copies

**Answered. Three parts of §2 and §3.3 are wrong.**

The real surface:

```cpp
StdLogosResult uploadUrl(const std::string &filePath, int64_t chunkSize, bool advertise)
StdLogosResult uploadInit(const std::string &filename, int64_t chunkSize, bool advertise)
StdLogosResult uploadChunk(const std::string &sessionId, const std::string &chunk)
StdLogosResult uploadFinalize(const std::string &sessionId)
StdLogosResult downloadToUrl(const std::string &cid, const std::string &filePath,
                             bool local, int64_t chunkSize, bool isPrivate, bool advertise)
StdLogosResult downloadChunks(const std::string &cid, bool local, int64_t chunkSize,
                              bool isPrivate, bool advertise)
StdLogosResult manifests()
StdLogosResult remove()
StdLogosResult setAdvertise(...)
```

Events: `storageUploadProgress`, `storageUploadDone` (carries `cid`),
`storageDownloadProgress`, `storageDownloadDone`, `storageDownloadManifestDone`,
`storageRemoveDone`, `storageConnect`, `storageStop`.

**There is no pin.** The closest thing is `setAdvertise()` — "Enable or disable
DHT announcements and serving to peers for a dataset. Published DHT records are
not withdrawn; they stop being republished and expire." So "the creator's node
pins every document" really means: the creator's node has the bytes in local
storage, does not call `remove()`, and keeps `advertise` on so other nodes can
fetch them. Nothing reserves space or guarantees retention. The spec's
conclusion is unchanged and arguably strengthened — the index pages on Storage
really are the only durable list — but the mechanism is not what it says.

**Listing exists, locally.** `manifests()` returns "all manifests stored
locally" with `cid`, `treeCid`, `datasetSize`, `blockSize`, `filename`,
`mimetype`. This is genuinely useful and §2 ruled it out: a creator can
enumerate what its own node holds, which gives the C4 "storage size" card
directly and makes a local reconciliation pass against the index possible.
There is still no network-wide listing, so the announce-on-a-topic design
stands.

**Deletion exists, locally.** `remove()` removes content "from local storage in
the background", and the deletion "may touch the network". This does **not**
make Collect's data retractable: the DHT record is not withdrawn, and any other
node that fetched a document keeps its copy. So the privacy reasoning behind
`rounded_1km` and behind hide-rather-than-delete is intact — a published exact
coordinate cannot be recalled — but §2's flat "nothing can be deleted" is
wrong, and `remove()` is the right call for a creator's `experiments.forget`.

**No copy count.** Nothing in the API reports how many nodes hold a CID. The C4
summary card cannot show "copies" as §3.3 describes. The honest substitutes are
the count the spec's own fallback suggests (copies *we know of*: creator,
submitter, mirror) or dropping the number.

**No fetch timeout.** Neither download method takes one; both are async with
completion events. The §3.3 "gives up after 10 min" has to be a timer on our
side that stops waiting for `storageDownloadDone`. Note `local` means "only
reads from locally cached data (no network)", which is the right flag for
re-reading our own documents without a network round trip.

**Size:** no per-file limit documented; config carries
`"storage-quota": 21474836480` (20 GiB total).

---

## Q4 — Delivery: retention, size, and RLN on testnet

**Answered, and this is the finding that changes the plan.**

```cpp
StdLogosResult send(const std::string &contentTopic, const std::vector<uint8_t> &payload)
StdLogosResult subscribe(const std::string &contentTopic)
StdLogosResult unsubscribe(const std::string &contentTopic)
StdLogosResult storeQuery(const std::string &jsonQuery, const std::string &peerAddr, int64_t timeoutMs)
StdLogosResult rlnState()
```

`send` builds `{ "contentTopic": string, "payload": base64, "ephemeral": false }`.
Received messages arrive on `messageReceived(messageHash, contentTopic, payload,
source, timestamp)` where **`source` is `"live"` or `"history"`** — history
meaning Store catch-up, which is on by default (`backfillEnabled`). That is
exactly what §3.2's "creator offline, submitter re-announces" needs, and it is
better than the spec assumed: a creator coming back up gets a backfill rather
than relying only on re-announcements.

`storeQuery` takes `timeStart` / `timeEnd` in **nanoseconds** since the epoch,
`paginationLimit`, and `includeData` (full messages vs hashes only). So the
creator can ask for what it missed instead of waiting.

### RLN is required on logos.test

> "every logos.test node now needs the RLN modules and an active membership
> (a payer with at least 2×10⁸ native LEZ on testnet) before start passes."

Needs `liblogos_rln_module` ≥ 0.10.0 and `liblogos_lez_rln_module` ≥ 4.2.1.
The node's `start` **fails** without a funded membership; the funding happens
between `createNode` and `start`. Epochs on logos.test are **600 s**.

This lands on Collect hard, and the problem is not the creator — it is the
submitter. §1 says a submitter is "any Basecamp user with the link", and §3.2
has the submitter's own node publishing a `submission` announcement. If
publishing needs a funded RLN membership, then "any Basecamp user can submit"
is false on logos.test, and acquiring a membership means testnet tokens — which
`CLAUDE.md` rules out for v1 ("no blockchain, no tokens").

Three ways out, none free, and this needs a decision rather than a guess:

1. **Assume Basecamp provides the membership.** If a Basecamp 0.3.0 profile
   already ships a funded membership, submitters inherit it and nothing
   changes. **This is the thing to check first, and it needs a node.**
2. **Submitters do not announce.** The submitter uploads to Storage and hands
   the creator the CID some other way — but there is no other way in v1
   without Delivery, and Chat is ruled out by `CLAUDE.md`.
3. **A non-RLN preset for v1.** Develop and demo against a preset where RLN is
   off, and treat logos.test as a later milestone.

The 600 s epoch is at least consistent with the spec's design: re-announcing
every 10 minutes is one message per epoch, which is the cheapest possible
cadence under any per-epoch quota.

### Size and retention

**~150 KiB is the effective ceiling**, and for a reason worth knowing: store
sync "reads one of at most `DefaultMaxWakuMessageSize`, 150 KiB (153,600
bytes), whatever `--max-msg-size` says", and a larger message that one node
archives "is never synced to any peer". Collect's announcements are tiny, so
this costs nothing — it only confirms that documents must go to Storage and
only CIDs over Delivery, which is what §2 already does.

**Retention is operator-configured**, not a platform guarantee: the Store
protocol "requires a PostgreSQL connection and a retention policy
configuration such as `size:1GB`". So §2's "up to 7 days" of re-announcing is
our own policy, and the spec's "Delivery is only the doorbell" is right.

---

## Q3 and Q5, incidentally

**Q3 (HTTP gateway for Storage): still open, leaning no.** Nothing in the
Storage API suggests an HTTP read path; it is a C++ module API over local IPC.
The spec's default — "Reading needs Basecamp or a Logos node" — stands.

**Q5 (map tiles in QML): unchanged, and the constraint is real.** Nothing here
lifts the UI sandbox, so tiles would have to come through a core module. The
spec's default (placeholder map plus typed coordinates) stands.

---

## New constraints the spec does not mention

These came out of `lgs basecamp docs` and matter before any code is written.

**1. `module_data/` does not survive a relaunch.** Everything under
`module_root` — including basecamp's per-module persisted state and its logs —
"do[es] not survive a relaunch. That is the clean-slate contract working as
intended — a module that needs state across launches must not keep it there."

This is the most consequential finding after RLN. Collect's creator node needs
to persist its identity key, its experiments, and its index state across
launches, or it stops being the indexer. `CLAUDE.md`'s `--user-dir` note is
right in spirit, but the docs clarify the mechanism: `--user-dir` and `launch`
are "the same data-tree redirect reached from two entry points", so the
workaround is hand-launching with `--user-dir` pointing **outside** the
scaffold profile tree that `launch` scrubs. Where `collect_core` should keep
durable state is a new open question.

**2. The four acceptance-test profiles are fine.** `basecamp setup` seeds only
`alice` and `bob`, but a project can declare its own under
`[basecamp.profiles.<name>]` in `scaffold.toml`, and "unknown profiles are
seeded on first launch". So §8's `creator`, `alice`, `bob`, `reader` all work —
they just have to be declared.

**3. A `ui_qml` package must ship a 256×256 PNG icon** bundled to
`assets/icon.png` at the package root, and `main` / `view` in `metadata.json`
must point at files that exist in the built variant. Validation rejects both.
Neither is in the handoff.

**4. macOS has a 104-byte socket-path budget.** liblogos opens a
`logos_token_<module>` Unix socket under `TMPDIR`; macOS caps
`sockaddr_un.sun_path` at 104 bytes and module loading aborts past it. Scaffold
defaults `runtime_dir` to `/tmp/lgs-<profile>` on macOS to stay under it. Worth
knowing before debugging a mystery load failure.

**5. Transitive `logos-module-builder` inputs must `follows` the top-level
pin.** Otherwise `flake.lock` carries two module-builder entries and the stale
one wins under scaffold's `--override-input`, so `nix build .#lgx` works
directly but fails through scaffold. Since `collect_core` and `collect_ui` are
sibling sub-flakes and will both pull in `delivery_module`, this applies to us.

**6. Sibling sub-flakes must use the single-line `path:../<sibling>` form.**
The scaffold parser is line-level and will not see a URL nested inside a
multi-line attrset.

**7. `logos-module-builder` must be exactly 0.3.0.** 0.3.1 "is already ahead of
it". Basecamp 0.3.0 speaks `logos-protocol` 0.9 and a module built against
0.2.x "may install cleanly but have its calls refused once loaded". The pin in
`CLAUDE.md` is right and load-bearing.

---

## Sources

- [Storage module API reference](https://logos-co.github.io/logos-storage-module/latest/api_reference.html)
- [Delivery module API reference](https://logos-co.github.io/logos-delivery-module/latest/pages/api_reference.html)
- [Build and run a Logos core module](https://docs.logos.co/core/build-modules/build-and-run-a-logos-core-module)
- [About logos-scaffold](https://docs.logos.co/build-an-app/scaffold/about-logos-scaffold)
- [logos-module-builder](https://github.com/logos-co/logos-module-builder)
- [logos-dev-boost spec.md](https://github.com/logos-co/logos-dev-boost/blob/master/docs/spec.md)
- [Delivery PR #149 — turn RLN on in the logos.test preset](https://github.com/logos-co/logos-delivery-module/pull/149)
- [Delivery issue #4485 — 150 KiB store-sync ceiling](https://github.com/logos-messaging/logos-delivery/issues/4485)
- `lgs basecamp docs` from logos-scaffold 0.4.0, installed locally
