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
  schemas/  examples/  tools/  design/
```

## Platform constraints

1. QML UI modules are sandboxed: no network, no files outside the module. All Storage, Delivery, file and crypto work happens in `collect_core`; the UI calls it with `logos.callModule`.
2. Logos Storage has no listing and no paid persistence. The creator's node pins every document of its experiments; submitters pin their own. The index pages on Storage are the durable list of submissions.
3. Delivery Store is temporary. Submitters re-announce until they get a receipt; the creator re-publishes `head` every 10 minutes.
4. Logos Chat addresses change on every launch; do not use Chat for anything here.
5. `lgs basecamp launch <profile>` wipes the profile. For tests that need persistence, run Basecamp with `--user-dir`.
6. Nix package builds see only git-tracked files: `git add` new files before `nix build .#lgx`.

## Rules

- Every document is validated against `schemas/collect.schema.json` before upload and after download. Invalid documents are never indexed.
- Sign canonical JSON (RFC 8785) without the `sig` field, Ed25519, using the module's identity key.
- Photos: read location and time, then remove all metadata before upload. Never upload the original file.
- Location rounding (`rounded_1km`) happens before anything is stored.
- The form is rendered from the protocol JSON. Never hard-code a protocol's fields in QML.
- Follow the design files in `design/canvas/` for copy and layout, and `design/Theme.qml` for tokens. 44 px targets; every control keyboard-reachable with an accessible name.
- Build the UI against fixtures (`COLLECT_FIXTURES=1`, data from `examples/`) first, then wire to live data.
- Tests: schema validation of examples, signing and verification, metadata stripping, rounding, index paging, window checks, rate limit. Run `python3 tools/validate.py` in CI.

## Do not

- Do not invent Logos APIs. Read first:
  - Storage: https://logos-co.github.io/logos-storage-module/latest/api_reference.html
  - Delivery: https://logos-co.github.io/logos-delivery-module/latest/pages/api_reference.html
  - Core modules: https://docs.logos.co/core/build-modules/build-and-run-a-logos-core-module
  - Scaffold: https://docs.logos.co/build-an-app/scaffold/about-logos-scaffold
- Do not add encryption, a blockchain, tokens or payments in v1.
- Do not collect or display personal data beyond the optional display name a submitter chooses.
- Do not change pins or the document formats without asking.
