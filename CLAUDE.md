# CLAUDE.md — Collect (Logos Basecamp module)

Standing instructions for every Claude Code session in this repository. The product spec is `SPEC.md`; the data contract is `schemas/collect.schema.json`. When the spec and the real Logos source disagree, the source wins: say so, then update `SPEC.md` in the same change.

## What we are building

A small Basecamp app. A creator defines an experiment with dates and one or more data collection forms (protocols). Submitters with Basecamp open the link and fill in a form. Every document is signed and stored publicly on Logos Storage; Logos Delivery carries announcements. No blockchain, no tokens, no encryption of content in v1.

## Pinned stack (do not upgrade without asking)

| Component | Pin |
| --- | --- |
| Logos Basecamp | 0.3.0 (Testnet v0.3) |
| logos-scaffold (`lgs`) | 0.4.0, commit `d4b15ff` |
| logos-module-builder | 0.3.0 (pin in every `flake.nix`) |
| Nix | flakes enabled |

Development: Linux or macOS. First `lgs basecamp setup` needs about 5 GB free memory.

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
1b. Storage is reached by name, not as a declared dependency: naming it in
   `metadata.json` fails the build because it publishes no LIDL contract our
   flake pins. The contract is committed at `docs/storage_module.lidl` — read
   it, not the rendered reference. Three traps it corrects: every event payload
   is a JSON **string** (`payload: tstr`), so it must be parsed and not
   indexed; `downloadToUrl` is **async** and its completion event carries no
   CID; and the node must be `init()`-ed and `start()`-ed by us.
2. Logos Storage has no network-wide listing and no paid persistence. There is **no pin operation**: keeping a document means its bytes are in local storage, `remove()` is never called, and `advertise` stays on. `manifests()` does list what the local node holds. `remove()` deletes locally only — the DHT record is not withdrawn and other nodes keep what they fetched, so nothing published is ever retractable. The index pages on Storage are the durable list of submissions.
3. Delivery Store is temporary and its retention is operator-configured. Messages cap out at ~150 KiB — never near mattering, since an announcement only names a CID. Submitters re-announce until they get a receipt; the creator re-publishes `head` every 10 minutes. Contract at `docs/delivery_module.lidl`. Its traps are **not** Storage's: events are typed and **positional**, `payload` is `bstr` (base64, decode with the SDK's `bytes` helper), and inbound messages **must** be deduplicated by `messageHash` because Store catch-up replays a restart's backlog. RLN: `rlnState()` reports it, and the presence of `registryId` is what says RLN applies — an unreadable answer counts as not-ready (SPEC Q9).
4. Logos Chat addresses change on every launch; do not use Chat for anything here.
5. `lgs basecamp launch <profile>` wipes the profile, including basecamp's per-module `module_data/`. A module needing state across launches must not keep it there (SPEC.md Q8). For tests that need persistence, hand-launch Basecamp with `--user-dir` pointing outside the scaffold profile tree.
6. Nix package builds see only git-tracked files: `git add` new files before `nix build .#lgx`. The flake root is the module directory, so nothing above it exists in the build — that is why the contract is copied into the module.
6b. `rust-lib/Cargo.lock` is committed and must be regenerated against the staged SDK after any dependency change:
    `nix build github:logos-co/logos-module-builder/0.3.0#rust-sdk-src -o logos-rust-sdk-src && (cd rust-lib && cargo generate-lockfile)`
7. Each sub-flake must expose `packages.<system>.lgx`, and every transitive `logos-module-builder` input must `follows` the top-level pin — otherwise `flake.lock` carries two and the stale one wins under scaffold's `--override-input`. Sibling inputs must use the single-line `path:../<sibling>` form; the scaffold parser is line-level.
8. A `ui_qml` package must ship a 256x256 PNG icon at `assets/icon.png`, and `main` / `view` in `metadata.json` must name files that exist in the built variant. Validation rejects both.
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
- Tests: schema validation of examples, signing and verification, metadata stripping, rounding, index paging, window checks, rate limit. Run `python3 tools/validate.py`, `tools/cargo.sh test` in `modules/collect_core/rust-lib/core`, and `modules/collect_ui/tests/run.sh` in CI. `tools/cargo.sh` exists because `~/.rustup` has no default toolchain; it pins the same cargo/rustc the Nix module build uses.

## Do not

- Do not invent Logos APIs. `docs/api-verification.md` records what was checked on 28 October 2026 and what is still open. Read first:
  - Storage: https://logos-co.github.io/logos-storage-module/latest/api_reference.html
  - Delivery: https://logos-co.github.io/logos-delivery-module/latest/pages/api_reference.html
  - Core modules: https://docs.logos.co/core/build-modules/build-and-run-a-logos-core-module
  - Scaffold: https://docs.logos.co/build-an-app/scaffold/about-logos-scaffold
- Do not add encryption, a blockchain, tokens or payments in v1.
- Do not collect or display personal data beyond the optional display name a submitter chooses.
- Do not change pins or the document formats without asking.
