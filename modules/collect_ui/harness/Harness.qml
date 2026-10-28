// Renders the screens outside Basecamp so they can be checked without a node.
//
// Not part of the module package. Run it with the `qml` runtime:
//   ./modules/collect_ui/harness/run.sh
//
// For looking at the screens. Automated verification lives in tests/ — run
// tests/run.sh, which checks the renderer without needing a window.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect
import "../src/qml/screens"
import "../src/qml/components"
import QtQuick.Layouts

ApplicationWindow {
    id: win

    width: 900
    height: 1000
    visible: true
    title: "Collect \u00b7 harness"
    color: Theme.ground

    // Inside the example experiment's window (20 Oct - 20 Nov 2026).
    readonly property double insideWindow: Date.parse("2026-10-28T08:00:00Z")

    property bool ready: false

    // The harness is a plain Qt app, so it may read the example documents off
    // disk and hand them to Core. Inside Basecamp the QML module cannot, which
    // is why Core takes them rather than fetching them.
    readonly property string fixtureRoot: "../../../examples/"

    Component.onCompleted: win.loadFixtures()

    function loadFixtures() {
        var names = { experiment: "experiment.json", protocol: "protocol.json",
                      submission: "submission.json", index: "index.json" };
        var docs = {};
        var pending = Object.keys(names).length;
        for (var key in names) {
            readJson(key, names[key], docs, function () {
                if (--pending === 0) {
                    Core.seed(docs);
                    win.ready = true;
                }
            });
        }
    }

    function readJson(key, name, docs, done) {
        var req = new XMLHttpRequest(); // qmllint disable unqualified
        req.onreadystatechange = function () {
            if (req.readyState !== XMLHttpRequest.DONE)
                return;
            try {
                docs[key] = JSON.parse(req.responseText);
            } catch (e) {
                console.error("could not read", name, "-", e,
                              "(run via harness/run.sh, which sets QML_XHR_ALLOW_FILE_READ)");
            }
            done();
        };
        req.open("GET", Qt.resolvedUrl(win.fixtureRoot + name));
        req.send();
    }

    // The screen under inspection.
    property string page: "join"

    header: ToolBar {
        RowLayout {
            anchors.fill: parent
            Repeater {
                model: ["join", "form", "submitted"]
                delegate: Button {
                    id: tab
                    required property var modelData
                    text: modelData
                    checked: win.page === modelData
                    onClicked: win.page = modelData
                }
            }
        }
    }

    Loader {
        id: stage
        anchors.fill: parent
        active: win.ready
        sourceComponent: {
            switch (win.page) {
            case "form":      return formPage;
            case "submitted": return submittedPage;
            default:          return joinPage;
            }
        }
    }

    Component {
        id: joinPage
        Join {
            experiment: Core.experimentDoc
            nowMs: win.insideWindow
        }
    }

    Component {
        id: formPage
        Form {
            experiment: Core.experimentDoc
            protocol: Core.protocolDoc
            accepting: true
        }
    }

    Component {
        id: submittedPage
        Submitted {
            protocol: Core.protocolDoc
            cid: "logos:zb2rhW4nExample9c71F"
            received: false
        }
    }
}
