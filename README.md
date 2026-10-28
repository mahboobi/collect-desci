# Collect — Basecamp module handoff

A simple Logos Basecamp module: a **creator** defines an experiment (title, description, start and end date) with custom data collection forms (**protocols**). **Submitters** open the link in Basecamp and fill in a form. Everything is signed and stored publicly on Logos Storage.

This is a new module, separate from Open Labbook. Start it in a new repository.

| Path | What it is |
| --- | --- |
| `CLAUDE.md` | Standing rules for Claude Code (stack pins, platform constraints, do-nots) |
| `SPEC.md` | The full spec: users, storage design, flows, field types, media rules, API, screens, acceptance tests, open questions |
| `schemas/collect.schema.json` | JSON Schema for the experiment, protocol, submission and index documents and the three Delivery messages |
| `examples/` | One valid example of each (Urban wildlife sightings) |
| `tools/validate.py` | Validates the examples and runs 9 negative tests (`pip install jsonschema`) |
| `design/canvas/` | The 7 design artboards (`.dc.html`, reference only) and their layout |
| `design/tokens.json`, `design/Theme.qml` | Visual tokens (same look as Open Labbook) |

## How to use

1. Create an empty repository (for example `collect`) and copy this folder's contents into it.
2. Open it in Claude Code and paste the prompt below.
3. Optional: export PNGs of the canvas artboards into `design/screenshots/` so Claude Code can look at them.

## Kickoff prompt

```text
We are building "Collect", a new Logos Basecamp module. Read CLAUDE.md and
SPEC.md, look through design/canvas/*.dc.html, and run tools/validate.py.

Step 1 — Verify before code. Inspect logos-module-builder 0.3.0 (templates
ui-qml-backend and rust) and the Storage and Delivery module API references.
Answer SPEC.md open questions Q1, Q2 and Q4 with exact call signatures, and
list anything in SPEC.md §2 or §6 that the real APIs cannot do. Propose the
repository layout. Wait for my go-ahead.

Step 2 — Spike (one week): scaffold collect_ui and collect_core; on two
scaffold profiles, creator uploads a signed protocol JSON to Logos Storage and
announces it on the experiment's Delivery topic; the second profile receives
the announcement, fetches the CID, validates it against the schema and shows
the protocol name. Paste the evidence.

Step 3 — Build in this order, one commit per step, fixtures first then live:
  1. Documents: canonical JSON, signing, schema validation, Storage upload and
     fetch, pinning.
  2. Creator flow: C1 home, C2 experiment (dates validation), C3 builder,
     publish manifest + head, link.
  3. Submitter flow: open link (S1), form renderer for all 9 field types (S2),
     media pipeline with metadata removal and location rounding, submit,
     announce and re-announce, S3 status.
  4. Creator indexer: receive, validate, window and rate checks, pin, index
     pages, receipts; C4 results, hide submitter, export CSV/JSON + media.
Run the acceptance tests in SPEC.md §8 on the profiles creator, alice, bob and
reader after steps 3 and 4.

Do not invent Logos APIs. If something in SPEC.md cannot work on the real
modules, stop, tell me, and update SPEC.md.
```

Design canvas: https://claude.ai/artifact/Py1FSWuuJhUhCbiBKnXNEy
