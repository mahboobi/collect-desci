# CLAUDE.md — Collect (Logos Basecamp module)

Standing instructions for every Claude Code session in this repository. The product spec is `SPEC.md`; the data contract is `schemas/collect.schema.json`. When the spec and the real Logos source disagree, the source wins: say so, then update `SPEC.md` in the same change.

## What we are building

A small Basecamp app. A creator defines an experiment with dates and one or more data collection forms (protocols). Submitters with Basecamp open the link and fill in a form. Every document is signed and stored publicly on Logos Storage; Logos Delivery carries announcements. An experiment may offer a LEZ TestToken reward per approved submission, paid by the creator after it ends (SPEC section 11).

Each person also has a **science profile** (top right) and each project has a **discussion** (SPEC section 10). Neither is ever written to Logos Storage: the profile stays on the device, shared profile fields are sealed per recipient (X25519 + ChaCha20-Poly1305, `core/src/seal.rs`) and travel on the experiment's Delivery topic, and messages travel on the same topic and are kept in a local history. Documents themselves stay unencrypted.

## Pinned stack (do not upgrade without asking)

| Component | Pin |
| --- | --- |
| Logos Basecamp | 0.3.0 (Testnet v0.3) |
| logos-scaffold (`lgs`) | 0.4.0, commit `d4b15ff` |
| logos-module-builder | 0.3.0 (pin in every `flake.nix`) |
| Nix | flakes enabled |

Development: Linux or macOS. First `lgs basecamp setup` needs about 5 GB free memory.

**Testing happens in the installed `/Applications/LogosBasecamp.app`** (0.3.2 on testnet, as of 29 October 2026), not in scaffold profiles. Build `nix build .#lgx-portable` in each module and install with a *portable* `lgpm` (the scaffold's is a `-dev` build and refuses portable packages). Never run a scaffold Basecamp at the same time: every Storage instance shares `~/.logos_storage/data`, and the second one fails with "Should create metadata store!". The modules are still built with logos-module-builder 0.3.0, and the app accepts them.

## Layout

```
collect/
  CLAUDE.md  SPEC.md  scaffold.toml
  modules/collect_ui/      # ui-qml-backend template, QML
  modules/collect_core/    # rust template
    metadata.json flake.nix CMakeLists.txt
    rust-lib/              # the crate named by codegen.rust.crate
      Cargo.toml Cargo.lock
      src/lib.rs           # the trait — the builder scans THIS file for it
      core/                # all the logic, and every test
        contract/collect.schema.json   # copy; schemas/ is the source of truth
  schemas/  examples/  tools/  design/  docs/
```

The split is forced, not stylistic. The builder scans `rust-lib/src/lib.rs`
for the trait, and the trait names types from a scaffold that only exists in a
Nix build — so that crate cannot be tested. The logic crate sits *inside*
`rust-lib/` because the builder stages only that directory; a sibling path dep
is not copied. See `docs/api-verification.md` round 3.

## Platform constraints

1. QML UI modules are sandboxed: no network, no files outside the module. All Storage, Delivery, file and crypto work happens in `collect_core`; the UI calls it with `logos.callModule`.
1a. The sandbox also refuses `data:` URLs (images included) and parents any
   QtQuick dialog to an offscreen window. Images reach the view as `file://`
   URLs into `plugins/collect_ui/cache/`, written by the C++ backend
   (`collect_ui_backend.cpp`); file choosing goes through the backend's
   `pickImage`. Never hand the view a `data:` URL directly.
1b. Storage and Delivery must be **declared** in `metadata.json` *and* given
   flake inputs with `follows` unification. Installing a module does not load
   it — the declaration is what makes the host load it, and without one every
   call returns `object_unavailable` with subscriptions stuck `DEFERRED`. The
   clients still call by name (`LogosModuleSDK::new().plugin(…)`) so
   `tools/clients-check` can compile them outside Nix; the declaration is for
   the loading, not the typed client. The contract is committed at `docs/storage_module.lidl` — read
   it, not the rendered reference. Three traps it corrects: every event payload
   is a JSON **string** (`payload: tstr`), so it must be parsed and not
   indexed; `downloadToUrl` is **async** and its completion event carries no
   CID; and the node must be `init()`-ed and `start()`-ed by us.
2. Logos Storage has no network-wide listing and no paid persistence. There is **no pin operation**: keeping a document means its bytes are in local storage, `remove()` is never called, and `advertise` stays on. `manifests()` does list what the local node holds. `remove()` deletes locally only — the DHT record is not withdrawn and other nodes keep what they fetched, so nothing published is ever retractable. The index pages on Storage are the durable list of submissions.
3. Delivery Store is temporary and its retention is operator-configured. Messages cap out at ~150 KiB — never near mattering, since an announcement only names a CID. Submitters re-announce until they get a receipt; the creator re-publishes `head` every 10 minutes. Contract at `docs/delivery_module.lidl`. Its traps are **not** Storage's: events are typed and **positional**, `payload` is `bstr` (base64, decode with the SDK's `bytes` helper), and inbound messages **must** be deduplicated by `messageHash` because Store catch-up replays a restart's backlog. RLN: `rlnState()` reports it, and the presence of `registryId` is what says RLN applies — an unreadable answer counts as not-ready (SPEC Q9).
4. Logos Chat addresses change on every launch; do not use Chat for anything here.
4a. **Never block a call on starting a node.** `collect_ui`'s backend calls
   `session_capabilities()` from its own `onContextReady`, so anything slow in
   a `collect_core` method stalls the ui-host until basecamp kills it with
   `Timeout waiting for ui-host ready signal`. Storage and Delivery connect on
   a connector thread and are collected without blocking.
4b. **`on_context_ready` is only for work needing no other module.** A
   module's own auth token is registered with the host *after* that callback
   returns, so a cross-module call from there fails `unauthorized` with
   "token not recognized (re-exchange failed)". Open files there; reach other
   modules lazily, on the first real call (`Collect::platform`).
5. `lgs basecamp launch <profile>` wipes the profile, including basecamp's per-module `module_data/`. A module needing state across launches must not keep it there (SPEC.md Q8). For tests that need persistence, hand-launch Basecamp with `--user-dir` pointing outside the scaffold profile tree.
6. Nix package builds see only git-tracked files: `git add` new files before `nix build .#lgx`. The flake root is the module directory, so nothing above it exists in the build — that is why the contract is copied into the module.
6b. `rust-lib/Cargo.lock` is committed and must be regenerated against the staged SDK after any dependency change:
    `nix build github:logos-co/logos-module-builder/0.3.0#rust-sdk-src -o logos-rust-sdk-src && (cd rust-lib && cargo generate-lockfile)`
7. Each sub-flake must expose `packages.<system>.lgx`, and every transitive `logos-module-builder` input must `follows` the top-level pin — otherwise `flake.lock` carries two and the stale one wins under scaffold's `--override-input`. Sibling inputs must use the single-line `path:../<sibling>` form; the scaffold parser is line-level.
8. A `ui_qml` package must ship a 256x256 PNG icon at `assets/icon.png`, and `main` / `view` in `metadata.json` must name files that exist in the built variant. Validation rejects both.
8b. Announcements need RLN, which is three modules deep: `delivery_module` ->
   `liblogos_rln_module` -> `liblogos_lez_rln_module`, the last two from
   `logos-rln-modules` and neither shipped with basecamp. Install the backend;
   do **not** declare the bridge as a Collect dependency — basecamp then walks
   the chain and refuses to load Collect itself. A module Collect never calls
   must not be able to stop Collect opening.
9. On macOS the module socket path is capped at 104 bytes, so keep `runtime_dir` short (`/tmp/lgs-<profile>` is the scaffold default).

## Rules

- Every document is validated against `schemas/collect.schema.json` before upload and after download. Invalid documents are never indexed.
- Sign canonical JSON (RFC 8785) without the `sig` field, Ed25519, using the module's identity key.
- Photos: read location and time, then remove all metadata before upload. Never upload the original file.
- Location rounding (`rounded_1km`) happens before anything is stored.
- The form is rendered from the protocol JSON. Never hard-code a protocol's fields in QML.
- Follow the design files in `design/canvas/` for copy and layout, and `design/Theme.qml` for tokens. 44 px targets; every control keyboard-reachable with an accessible name.
- Build the UI against fixtures (`COLLECT_FIXTURES=1`, data from `examples/`) first, then wire to live data. `COLLECT_FIXTURES` belongs to `collect_core`: the QML sandbox cannot read `examples/`, so fixture and live data reach the UI the same way, through `Core.qml`.
- Set `QT_QUICK_CONTROLS_STYLE=Basic`. The design system customizes `background` and `contentItem` on every control, which the macOS native style refuses — controls silently lose their styling otherwise.
- A QML property may not be named `on<Something>`: it parses as a handler for a signal `<something>`. `design/Theme.qml` shipped with `onAccent`, which made the file unloadable; it is `textOnAccent` now, while `design/tokens.json` keeps the `onAccent` key.
- A CID is taken over the **canonical JSON of the signed document**, `sig`
  included — the only representation two nodes are guaranteed to agree on. A
  CID written into a document must match `^logos:[A-Za-z0-9]+$`; validate and
  prefix the raw address rather than trusting it (`store::as_document_cid`).
- A signed document cannot be edited. Changing a manifest (attaching a form,
  editing details) makes a **new version**: bump `version`, set `prev` to the
  CID the old version was published to, re-sign, re-validate, re-publish.
  `Session::reseal` is the only place that does this.
- Tests: schema validation of examples, signing and verification, metadata stripping, rounding, index paging, window checks, rate limit. Run the validator through nix, since the system python has no `jsonschema`:
  `nix-shell -p 'python3.withPackages (ps: [ ps.jsonschema ])' --run 'python3 tools/validate.py'`.
  Then `tools/cargo.sh test` in `modules/collect_core/rust-lib/core`, and `modules/collect_ui/tests/run.sh` in CI. `tools/cargo.sh` exists because `~/.rustup` has no default toolchain; it pins the same cargo/rustc the Nix module build uses.

## Do not

- Do not invent Logos APIs. `docs/api-verification.md` records what was checked on 28 October 2026 and what is still open. Read first:
  - Storage: https://logos-co.github.io/logos-storage-module/latest/api_reference.html
  - Delivery: https://logos-co.github.io/logos-delivery-module/latest/pages/api_reference.html
  - Core modules: https://docs.logos.co/core/build-modules/build-and-run-a-logos-core-module
  - Scaffold: https://docs.logos.co/build-an-app/scaffold/about-logos-scaffold
- Payments exist in one place only: rewards (SPEC section 11, decided 30 October 2026, which lifted the v1 no-payments rule). LEZ TestToken on the LEZ testnet, paid by the creator through the `lez_core` wallet with `transfer_public`. Rules that must hold:
  - Nothing pays on its own. Every payment follows the creator ticking a confirmation on the Pay rewards screen, and only after `ends_at`.
  - `rewards.pay` pays one submitter per call and re-checks that row against a freshly built plan; a stale plan is refused.
  - A payout is written to `state.json` as `sending` **before** the transfer; an unknown outcome blocks further payments until the creator resolves it. Never pay a CID that any non-failed payout names.
  - `lez_core` is called by name and never declared in metadata.json (8b). Collect never opens, creates or unlocks a wallet, and never sees a key.
  - Approvals are local to the creator's node, until the creator presses "Push to Logos Storage": the published `collect/results@1` holds approved rows only, so its contents reveal which were approved (decided 30 October 2026).
- Encryption is for sealed profile fields only (`seal.rs`). Do not add another scheme, and do not encrypt documents.
- Personal data lives only in the profile. It never goes into a document, never onto Logos Storage, and never onto Delivery in the clear: each field leaves the device only when its own `share` switch is on, sealed for each member separately. Submissions still carry only the optional public display name and, when the submitter ticks the box on that form, their wallet address (decided 30 October 2026). The address is read from their own profile in `collect_core`, never taken from the UI, and the box starts unticked on every form.
- Do not put anything in a `message` that is not the person's own typed text: messages are readable by anyone with the experiment's link.
- Do not change pins or the document formats without asking.
