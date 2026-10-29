# Step 1 — the real Logos APIs, checked against SPEC.md

Verified 28 October 2026 against the published API references, the
`logos-module-builder` README, and `lgs basecamp docs` from the locally
installed `logos-scaffold` 0.4.0 (which prints its own embedded copy of the
compatibility contract, so it is the most authoritative source here).

**Updated the same day** after finding Nix *was* already installed (2.35.4,
flakes on, 24 GB store). The templates and the Rust SDK source were fetched and
read, which settled the open half of Q1 and most of Q8 — see "Round 2" below.

**Still not verified by running anything.** No module was built and no profile
was launched, so items marked **needs a node** remain open.

---

## Summary of what changed in SPEC.md

| SPEC claim | Reality | Where |
| --- | --- | --- |
| "Storage has no 'list' ... operation" | `manifests()` lists what **this node** holds. No network-wide listing. | §2 |
| "nothing can be deleted" | `remove()` deletes from **local** storage. The network copy and DHT record cannot be withdrawn. | §2, §3.3 |
| "pins every document" | There is no pin operation. Persistence is "stored locally and not removed"; `setAdvertise()` only controls DHT announcement. | §2 |
| C4 shows "storage size and **copies**" | No API returns a replication count. | §3.3 |
| "gives up after 10 min" | Storage takes no timeout, but every generated call has one (`_with_timeout`, default **20 s**) — round 2. | §3.3 |
| `logos.callModule(..., [jsonString])` returning `{ok, data}` | `callModule` exists, but the `{ok, data}` envelope is an app convention, not the platform's. Typed methods are the platform contract. | §6 |
| Announcements just publish to a topic | **logos.test requires a funded RLN membership before the node starts.** | §2 |
| Re-announce "up to 7 days" | Delivery Store retention is operator-configured, not guaranteed. | §2 |
| (not stated) | Effective message size cap is ~150 KiB. | §2 |
| (not stated) | `lgs basecamp launch` scrubs per-module state; the platform itself gives `instance_persistence_path` — round 2. | Q8 |
| §6's `callModule` shape for the UI's own backend | `ui-qml-backend` uses a typed `.rep` replica and `logos.watch`, not `callModule` — round 2. | §6 |
| (not stated) | Rust boundary types are `String`/`i64`/`u64`/`f64`/`bool`/`Vec<u8>`/`Option<T>` only, so JSON-in-a-String is forced, not chosen — round 2. | §6 |

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

**Settled in round 2** by reading the template itself — see below.

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

**No fetch timeout in the Storage API itself** — both download methods are
async with completion events. But round 2 found the call layer has one:
`_with_timeout` on every generated method, protocol default 20 s. So §3.3's 10
minutes is expressible, and the default would otherwise cut it off far too
early. `local` means "only reads from locally cached data (no network)", which
is the right flag for re-reading our own documents without a network round
trip.

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


---

# Round 2 — read from the templates and the Rust SDK

Nix turned out to be installed already. `nix flake init -t
github:logos-co/logos-module-builder/0.3.0#rust` and `#ui-qml-backend`, plus
`nix build …#rust-sdk-src`, answer what the docs did not.

All six templates exist as `CLAUDE.md` says: `default`, `rust`,
`rust-with-external-lib`, `ui-qml`, `ui-qml-backend`, `with-external-lib`.

## Q1, the Rust half — a trait *is* the contract

There is no `.lidl` to write and no `build.rs`. From the template's own header:

> "Rust-FIRST authoring: the trait below IS the contract. The builder derives
> the `.lidl` from it (`codegen.rust.trait` in metadata.json), generates the
> C-ABI scaffold around it, and compiles this crate to a staticlib."

```rust
pub trait MinimalRustModule: Send + 'static {
    fn greet(&mut self, name: String) -> String;
    fn get_status(&mut self) -> String;
    fn on_context_ready(&mut self, _ctx: &RustModuleContext) {}
}

pub trait MinimalRustModuleEvents {
    fn greeted(&self, greeting: String);
}

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/generated/provider_gen.rs"));

#[no_mangle]
pub extern "Rust" fn logos_module_install() { install::<Minimal>(); }
```

Things that constrain `collect_core` directly:

- **The trait name is derived, not chosen.** Module name in PascalCase plus
  `Module` unless it already ends in it: `collect_core` → `CollectCoreModule`.
- **`Send + 'static` is required** — generated dispatch stores the impl as
  `Box<dyn Any + Send>`.
- **Only these types cross the boundary:** `String`, `i64`, `u64`, `f64`,
  `bool`, `Vec<u8>`, `Option<T>`. No structs, no maps. **This vindicates §6's
  JSON-string convention — it is not a stylistic choice, it is the only way to
  pass a protocol or a submission across the ABI.**
- `crate-type = ["staticlib"]`, and an empty `[workspace]` table so cargo does
  not adopt a parent workspace.
- `logos-rust-sdk` is a path dependency at `../logos-rust-sdk-src`, staged by
  the builder from the same revision its generator comes from.
- Events are a second trait; each method becomes a free `emit_<name>(...)`,
  taking string arguments **by reference**.

## Calling Storage and Delivery from Rust

Dependencies listed in `metadata.json` get typed clients on `modules()`, and
the host auto-loads them:

```rust
modules().counter_module.increment(1).unwrap_or(-1)
modules().counter_module.increment_async(1, |res| { /* … */ });
```

Events from a dependency come back as an iterable `EventSubscription` that
outlives the proxy, so it can move into a listener thread — which is how
`collect_core` will consume `messageReceived` and `storageDownloadDone`.

**This also answers the fetch-timeout question properly, and better than round
1 did.** Every generated call has a timeout — the protocol default is **20
seconds** — with a `_with_timeout` twin:

```rust
pub fn fetch_with_timeout(&self, sym: &str, timeout: Duration) -> Result<Quote, LogosError>;
```

So §3.3's "gives up after 10 min" is `..._with_timeout(…,
Duration::from_secs(600))`, not a hand-rolled timer. The bound belongs to the
call, nothing is stored, and a duration below 1 ms or past `c_int::MAX` ms is
**refused** rather than clamped (`LogosError::InvalidTimeout`). Worth noting
the default is 20 s: a Storage fetch left on the default would give up long
before the spec's 10 minutes.

`current_caller()` is also available, with `caller.is_module("…")` — useful if
`collect_core` ever needs to refuse a call from anything but `collect_ui`.

## Q8 — the persistence path exists

```rust
pub struct RustModuleContext {
    pub module_path: String,
    pub instance_id: String,
    pub instance_persistence_path: String,
}
```

> "`instance_persistence_path` is a writable, per-instance directory — the
> place to keep a module's on-disk state."

Read via the generated `context()`, which returns `None` until the host stamps
it, so `on_context_ready` is where to open state.

**So Q8 is much smaller than it looked.** The platform does give a module a
proper durable directory. The scrub described in `lgs basecamp docs` is done by
`lgs basecamp launch`, not by Basecamp itself — it is the scaffold dev-loop's
deliberate clean-slate contract. In normal use the creator's state persists.

What remains is a **testing** problem, not a design one: SPEC §8 test 3 needs
the creator to stop and restart with its index intact, and `launch` will wipe
it. Use a hand-launched Basecamp with `--user-dir` outside the scaffold profile
tree, exactly as `CLAUDE.md` says.

## Q1, the QML half — §6's assumption is wrong for `ui-qml-backend`

The template is a `.rep` interface, a C++ backend, and a QML view:

```
class UiExample
{
    SLOT(int add(int a, int b))
    PROP(QString status="Ready" READONLY)
}
```

```qml
readonly property var backend: logos.module("ui_example")
readonly property string status: backend ? backend.status : ""

logos.watch(backend.add(a, b), function (v) { … }, function (e) { … });

Connections {
    target: logos
    function onViewModuleReadyChanged(moduleName, isReady) { … }
}
Component.onCompleted: { logos.isViewModuleReady("ui_example") }
```

So a `ui-qml-backend` module does **not** reach its backend with
`callModule(name, method, [json])`. It gets a typed QTRO replica: `PROP`s
auto-sync as QML properties, `SLOT`s are called directly and their pending
reply is delivered through `logos.watch(call, onValue, onError)`.

`metadata.json` for the UI side also requires `"type": "ui_qml"`, `"interface":
"universal"`, an `icon` (the 256×256 PNG), a `view` pointing at the QML entry,
and `codegen.rep`.

### A design question this raises

`collect_ui` as `ui-qml-backend` means: QML → `.rep` replica → C++ backend →
`callModule("collect_core", …)` → Rust. Since every piece of logic already
lives in `collect_core`, that C++ backend would be a pass-through whose only
job is restating each `collect_core` method as a `SLOT`.

The `ui-qml` template (QML-only, no C++ backend) would let QML call
`logos.callModule("collect_core", method, [json])` and
`callModuleAsync(..., cb)` directly, deleting that layer. `CLAUDE.md` pins
`ui-qml-backend`, so this is **not** a change to make unilaterally — raising it
as Q12.

What `ui-qml-backend` buys that `ui-qml` does not: process isolation, and
auto-syncing `PROP`s, which suit the long-lived status surfaces (S3's
signed/stored/announced/received ladder, C4's summary cards) better than
polling a JSON method would.


---

# Round 3 — it builds

`nix build .#lgx` in `modules/collect_core` produces a real package. This is
the first thing here verified by execution rather than by reading.

```
logos-collect_core-module-lib.lgx   7.0 MB
  manifest.json
  variants/darwin-arm64-dev/collect_core_plugin.dylib
```

`manifest.json` carries `manifestVersion 0.6.0`, `type: core`, and the
`hashes` block (`root`, `variants`, `variants/<variant>`) that `lgpm` validates
on install — the thing tutorial-era tooling lacked. Note the variant is
**`darwin-arm64-dev`**: `.#lgx` builds the dev variant, so a release package
needs a different attribute.

## The derived contract

The builder extracted this from the trait, with no `.lidl` written by hand:

```
module collect_core {
  version "0.1.0"
  depends []

  method session_capabilities() -> tstr
  method documents_canonicalize(request: tstr) -> tstr
  method documents_validate(request: tstr) -> tstr
  method documents_verify(request: tstr) -> tstr
  method protocols_validate_draft(request: tstr) -> tstr
  method submissions_validate(request: tstr) -> tstr
  method submissions_prepare_values(request: tstr) -> tstr
  method submissions_process_media(request: tstr) -> tstr
  method index_accept(request: tstr) -> tstr
  method index_page(request: tstr) -> tstr
  method geo_round(request: tstr) -> tstr
  method experiments_parse_link(request: tstr) -> tstr
  method call(method: tstr, request: tstr) -> tstr

  event submission_received(experiment: tstr, protocol: tstr, cid: tstr)
  event receipt_received(cid: tstr)
  event media_processed(draft_id: tstr, field_key: tstr, result: tstr)
  event sync_status_changed(status: tstr)
}
```

Two things confirmed by their absence: `on_context_ready` is **not** published
(framework plumbing, as the template says), and every parameter is `tstr` —
there was never an option to pass a struct.

## What the build actually required

Six things, none of them documented anywhere public. Each cost a failed build.

**1. The trait must be in `src/lib.rs` of the crate named by
`codegen.rust.crate`.** The generator extracts the contract by scanning that
one file:

> `Failed to extract contract from .../rust-lib/src/lib.rs: trait CollectCoreModule not found in source`

So the trait cannot be moved to a submodule, and cannot be behind a `cfg`.

**2. That crate cannot be built or tested outside Nix.** The trait's
`on_context_ready` names `RustModuleContext`, which only the injected
`generated/provider_gen.rs` defines. Combined with (1), the module crate is
necessarily a shim. Collect's logic lives in a separate crate — all 222 tests
run there with no Logos dependency — and the shim forwards to it.

**3. The builder stages only that crate's directory.** A path dependency on a
sibling (`path = "../core"`) is simply not in the build tree:

> `failed to read .../logos-collect_core-rust-src/core/Cargo.toml`

So the logic crate lives *inside* `rust-lib/` as `rust-lib/core/`.

**4. A nested crate needs `exclude`.** Two crates each declaring `[workspace]`
in one tree is an error (`multiple workspace roots found in the same
workspace`). The outer declares `[workspace] exclude = ["core"]`, which keeps
the inner crate independently testable — it must not need the staged SDK.

**5. `rust-lib/Cargo.lock` must be committed**, and generated against the
staged SDK, because the SDK is a path dependency that does not exist until it
is staged:

```bash
nix build github:logos-co/logos-module-builder/0.3.0#rust-sdk-src -o logos-rust-sdk-src
(cd rust-lib && cargo generate-lockfile)
```

**6. The module crate needs `serde_json`** even when nothing hand-written in it
does. The generated `emit_*` functions build their argument arrays with
`serde_json::Value`.

## The flake root is the module, not the repository

`src = ./.` means the module directory. Anything above it does not exist in the
build, which is why `schemas/collect.schema.json` could not be `include_str!`d
and the module carries `rust-lib/core/contract/collect.schema.json` instead.
`schemas/` stays the source of truth; a test asserts the two are byte-identical
and fails if they drift. That test only compiles under `cfg(test)`, where the
repository path *is* reachable — which is exactly where a drift would be
introduced.

## Still not verified

Nothing has been **loaded**. The package builds and validates structurally, but
`lgs basecamp setup` has not run, so no module has been installed into a
profile and no call has crossed the real ABI. Q9 (RLN membership) and the
whole Storage/Delivery surface remain untouched.


---

## Round 4 — the Storage contract itself, 29 October 2026

Rounds 1-3 read the rendered API reference. This round reads the contract the
module publishes:

```
nix build github:logos-co/logos-storage-module#lidl
```

It is committed as `docs/storage_module.lidl` (storage_module 3.0.2, flake rev
`7548f41`). Everything below is from that file, and three items contradict what
the prose implied.

**1. Every event payload is `tstr`, not an object.** The contract is
`event storageUploadDone(payload: tstr)`, and the JSON shape in the description
is the shape *of that string*. So an event arrives as the argument array holding
one JSON **string**, and reading it as an object yields null for every field —
which is indistinguishable from an upload that never completed. This is the
single most dangerous detail here, because getting it wrong fails silently as a
timeout. `rust-lib/src/storage.rs::payload_of` is the parse, and
`an_event_payload_is_parsed_out_of_its_json_string` is the guard.

**2. `downloadToUrl` is asynchronous.** Round 2 treated the `_with_timeout`
call bound as the fetch bound. It is not: the call returns a session id as soon
as the request is accepted, and the file is written later, with completion on
`storageDownloadDone`. A `get` that reads the file when the call returns reads
nothing. Note also that `storageDownloadDone` carries **no `cid`** — only
`success`, `sessionId` and `error` — so a download has to be keyed on the
session id (which the contract says equals the CID).

**3. The node has to be started, and that is our job.** `init(cfg)` then
`start()`, with completion on `storageStart` (whose payload reports its reason
in `message`, not `error`). Nothing else in Basecamp necessarily starts it.
`init` is documented as once-per-instance, so a second call failing is expected
rather than fatal. `loadConfigOrDefault()` returns the module's own saved
configuration or its defaults — which is what we pass, rather than inventing a
config, because it carries the network, data directory and quota.

**4. There is a synchronous CID path, and we are not using it.**
`uploadInit` (sync, returns a session id) -> `uploadChunk` -> `uploadFinalize`
(**sync, returns the CID**) would avoid all event-waiting. It moves each chunk
as `tstr`, though, which is wrong for photo bytes, and the same store carries
media later. One path for documents and media is worth the event plumbing.

**5. Confirmations.** No pin (only `setAdvertise`, and "published DHT records
are not withdrawn; they stop being republished and expire"). `remove(cid)` is
local and background, with `storageRemoveDone`. `manifests()` lists local
holdings only. `exists(cid)` is synchronous. Quota 20 GiB, chunk default 65536.

### Declaring the dependency: what the builder actually requires

Putting `storage_module` in `metadata.json` `dependencies` **fails the build**:

```
error: metadata.json: module 'collect_core' lists dependencies that publish no LIDL
contract: storage_module
...
Fix: pass the flake input for each name above and re-pin it against a current
logos-module-builder ... For a target whose contract you do not want to pin at
all, drop the declaration and call it by name through modules().dynamic("<name>").
```

So a declared dependency needs a flake input publishing
`packages.<system>.lidl`. Both `logos-storage-module` and
`logos-delivery-module` publish one.

**Calling by name instead is not an option, and this cost a relaunch to
learn.** Dropping the declaration builds fine and the by-name proxy is the
right API — but *nothing then loads the dependency*. With `storage_module`
installed in the profile and `collect_core` running, every call came back:

```
collect_core: not publishing: Logos Storage is not available:
  storage_module not reachable: Method call storage_module.isRunning() failed:
  {"code":"object_unavailable","message":"failed to acquire remote object
   'storage_module' (module not loaded, not published, or transport failure)"}
```

with the event subscriptions reported as `DEFERRED, not lost; it will arm when
the module appears`. Installing a module does not load it; **the declaration in
`metadata.json` is what makes the host load it.** So both are declared, with
their transitive `logos-module-builder` unified onto our pin via `follows` —
the same trap `collect_ui` has for `collect_core`.

The hand-written clients still call by name through
`LogosModuleSDK::new().plugin("…")` — the same thing a generated client's
`::new()` does — because that keeps `tools/clients-check` able to compile them
outside the Nix build, which is what caught all three mistakes above. The
declaration is there for the loading, not for the typed client.

### The dev Basecamp has no storage_module

`logos-basecamp-0.3.0` as built by the scaffold ships four core modules:
`capability_module`, `modules_state`, `package_manager`, `package_downloader`.
The installed `/Applications/LogosBasecamp.app` additionally has
`storage_module`, but that is the non-dev variant and cannot be mixed with our
`-dev` modules (see `tools/load.sh` VARIANTS). So Storage has to be built from
the flake for the dev variant and installed into the profile like any other
module.


---

## Round 5 — the Delivery contract, 29 October 2026

```
nix build github:logos-co/logos-delivery-module#lidl
```

Committed as `docs/delivery_module.lidl` (delivery_module 0.3.3).

**Delivery's events are shaped unlike Storage's, and this is the trap.**
Storage declares every event as `payload: tstr` — one JSON string to parse.
Delivery declares typed, positional fields:

```
event messageReceived(messageHash: tstr, contentTopic: tstr,
                      payload: bstr, source: tstr, timestamp: int)
```

So the event data is an argument **list**, read by position, and `payload` is
`bstr` — bytes, base64 across the FFI boundary, decoded with the SDK's
`logos_rust_sdk::bytes::decode`. Code that assumed the Storage shape here finds
null everywhere. Both shapes now have a test naming the mistake they prevent
(`a_received_message_is_read_positionally_and_its_payload_decoded`).

**Deduplication is mandatory, and the contract says so.** `messageReceived`
carries `source: "live" | "history"`; Store catch-up (`backfillEnabled`,
default **true**) replays what was missed while the node was down, and upstream
suppresses duplicates "only for a few minutes and only in memory". A creator
that indexed every delivery would re-index its whole backlog after any restart.
`delivery::Seen` is the guard, bounded, with `indexer::accept`'s duplicate rule
still behind it.

**Q9 is answerable rather than assumed.** `rlnState()` returns
`{state, message}` plus `registryId`, `rlnIdentifier` and `epochSizeSec`
**once a preset has enabled RLN** — so the presence of a registry is what says
RLN applies at all, which is more durable than matching on state names that are
upstream's to rename. `rlnStateChanged` is the same transition as an event.
There is also an RLN *bridge* (`rlnBridgeEnable`, `rlnRespond`, and the
`dispatchRln*RequestEvent` events, with `liblogos_rln_module` as an optional
dependency), so a module can answer RLN requests on Delivery's behalf. Collect
does not: it reads the state and reports it.

An unreadable `rlnState` is treated as **required and not ready**. Claiming a
send will work and having it fail silently is worse than saying we cannot tell.

**Sends are not awaited.** `send` returns a request id and completes through
`messageQueued` / `messagePropagated` / `messageSent` / `messageError`. None of
those is the confirmation that matters: a submitter re-announces until the
*creator* sends a receipt (SPEC 3.2), so `messageSent` would only report that
the network took a message whose real acknowledgement is a different message.
What is checked up front is `rlnState()`, because that is the failure a person
can act on.

**Node setup.** `createNode(cfg)` then `start()`, completing on
`nodeStarted(success, message, timestamp)`. The layered config shape is
`{ "mode": "Edge", "preset": "logos.test" }` — `Edge` because Collect is a
client and has no business relaying other people's traffic. `anonymityLevel` is
left at its default: mix routing would change what the spec says about privacy,
and SPEC 2 forbids adding that in v1 without asking.

### The content topic

`/collect/1/<experiment id>/json` — Waku's four-part shape. One topic per
experiment, so a creator is not handed every other experiment's traffic. An
experiment id is already a public, non-guessable 16 characters, so the topic
leaks nothing a link does not.

### Inbound messages do not call into the session

Delivery hands messages to a listener thread; the session is `&mut`-owned by
the module. A callback would need the session behind a lock held across the
whole of `deliver`, which does Storage fetches and publishes inside it. So the
courier owns a bounded inbox and the session drains it on a timer
(`delivery.drain`), which keeps every mutation on one thread and makes the
whole receive path testable without one.


---

## Round 6 — loading and authorization, 29 October 2026

Two failures found by actually running it, each of which looked like the other
until the log was read closely.

### 1. Installing a module does not load it; the declaration does

`lgpm install` put `storage_module` in the profile and `lgpm list` showed it.
It was still never loaded, and every call returned:

```
{"code":"object_unavailable","message":"failed to acquire remote object
 'storage_module' (module not loaded, not published, or transport failure)"}
```

with the subscriptions reported as `DEFERRED, not lost; it will arm when the
module appears`. **The `dependencies` entry in `metadata.json` is what makes
the host load a module.** Both are declared now, with flake inputs whose
transitive `logos-module-builder` `follows` our pin.

Round 4's conclusion — "call it by name instead" — was right about the API and
wrong about what it bought. The clients still call by name, because that keeps
`tools/clients-check` able to compile them outside the Nix build. The
declaration is for the loading.

### 2. `on_context_ready` is too early for a cross-module call

With both modules loading, the calls failed differently:

```
[storage_module] ModuleProxy: rejecting unauthorized call to "isRunning"
                 - auth token not recognized
[collect_core]   LogosAPIClient: token for "storage_module" rejected by
                 provider; re-exchanging and retrying once
[collect_core]   not publishing: ... {"code":"unauthorized","message":"call to
                 'storage_module' rejected: token not recognized
                 (re-exchange failed)"}
```

The timestamps are the whole explanation. Those calls are at 12:15:32.811-814.
`LogosAPIConsumer: Informing module token for module: "collect_core"` is
**after** them, and `Module loaded: collect_core` is at .814. So a module's own
auth token is registered with the host *after* `on_context_ready` returns: a
cross-module call from there presents a token the provider has never seen, and
the SDK's automatic single re-exchange cannot help because there is nothing yet
to re-exchange against.

**So `on_context_ready` is only for work that needs no other module.** Collect
opens its signing key there — a file in a directory the host just handed us —
and attaches Storage and Delivery lazily on the first real call, by which time
the exchange has happened (`Collect::platform`). A failed attempt is retried on
a later call rather than being permanent, since "not loaded yet" is a state
that passes.

### Fail fast on the probe

Both clients now probe with a 3 s bound (`isRunning`, `getConnectionStatus`)
instead of the 20 s protocol default, and Delivery **aborts on a failed probe**
rather than continuing to `createNode` and `start`. The first run through that
code spent a full minute timing out against a module that had already said it
could not be reached — visible in the log as a 60-second gap between the
Storage and Delivery failures.
