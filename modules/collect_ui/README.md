# collect_ui

The QML side of Collect. Screens, the form renderer, and the one place that
talks to `collect_core`.

```
src/qml/
  Main.qml              the view entry point (metadata.json "view")
  collect/              the `collect` QML module
    qmldir
    Theme.qml           copied from design/Theme.qml — edit it there
    Core.qml            the only thing that talks to collect_core
  components/
    FormRenderer.qml    a whole form, from protocol JSON
    FieldRow.qml        one field: label, required marker, help, delegate, error
    Badge.qml  PublicNotice.qml
  fields/               one delegate per field type, all nine
  screens/
    Join.qml            S1
    Form.qml            S2
    Submitted.qml       S3
harness/                look at the screens without a node
tests/                  qmltestrunner suite
```

## Running it

```bash
./tests/run.sh       # 21 checks on the form renderer, headless
./harness/run.sh     # opens a window showing S1, S2, S3
```

Both find Qt 6 in the nix store on their own; set `QT_QML_ROOT` to override.

## The rule that shapes this

`CLAUDE.md`: the form is rendered from the protocol JSON, never hard-coded.
`FormRenderer` reads `protocol.fields` and picks a delegate by `type`. Nothing
under `fields/` knows what a wildlife sighting is, and a protocol naming a type
this build does not have shows "update Collect" rather than a text box, so an
unknown field cannot quietly collect the wrong shape.

## Validation is deliberately thin here

Each delegate reports `blank` and `problem` separately, mirroring
`collect_core::fields`: an untouched field reads as unanswered, not as an
error. That decides whether Submit looks enabled, and nothing more.

`collect_core::fields::validate_values` is authoritative. It runs again on
submit, and the creator runs it a third time on receive. A rule implemented
here *as well as* there is a rule that can drift, so this layer stays
immediate-feedback only.

The one exception is rounding, which is duplicated on purpose:
`LocationField.cellCentre` mirrors `geo::round_1km` so the submitter can see
which coordinate is about to become public *before* submitting. `collect_core`
rounds again and its answer is the one that gets signed.

## Not wired yet

`Core.qml` holds the documents the screens read and exposes `seed()`. What
fills it depends on SPEC **Q12** — whether `collect_ui` keeps a C++ backend
(`logos.module("collect_ui")` with a `.rep` replica) or becomes a `ui-qml`
module calling `logos.callModule("collect_core", ...)` directly. `src/*.rep`
sketches the first; `src/collect_ui_backend.{h,cpp}` is the piece still
missing. Either way the screens and the renderer are unaffected — they read
from `Core`.

The creator screens (C1 home, C2 experiment, C3 builder, C4 results) are not
built yet. The submitter flow S1 -> S2 -> S3 is.
