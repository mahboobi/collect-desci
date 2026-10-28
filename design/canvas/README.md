# Design source (reference only)

Artboards from the Claude design canvas "Collect — Basecamp module" (28 October 2026). Copy, layout and states are exact; the app is built in QML.
`{{name}}` placeholders are filled by `renderVals()` at the bottom of each file; read it for data shapes and interactive state.
`support.js` (the canvas runtime) is not included, so files do not render standalone.

| File | Screen |
| --- | --- |
| Main.dc.html | C1 Home (Created by you / You submit to) |
| Experiment.dc.html | C2 Create or edit experiment |
| Builder.dc.html | C3 Protocol builder |
| Results.dc.html | C4 Results |
| Join.dc.html | S1 Experiment page opened from a link |
| Form.dc.html | S2 Submission form (prop startDone shows S3) |
| Submitted.dc.html | S3 Submitted (imports Form) |
