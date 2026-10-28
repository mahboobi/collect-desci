// The whole app: dashboard, experiment detail, the submitter flow, and
// creating an experiment.
//
// Routing lives here so each screen stays a view over Core and emits signals
// rather than navigating itself.
pragma ComponentBehavior: Bound

import QtQuick
import collect
import "screens"

Item {
    id: app

    // Lets a test place itself inside or outside an experiment window.
    property double nowMs: Date.now()

    // home | detail | form | submitted | new
    property string page: "home"
    property string openExperimentId: ""
    property bool openMine: false
    property var activeProtocol: null
    property string lastCid: ""
    property bool receiptSeen: false
    property var createProblems: ({})

    // The typed replica for this module's own backend. Null until the view
    // module is ready, so every read is guarded.
    readonly property var backend: logos.module("collect_ui") // qmllint disable unqualified

    anchors.fill: parent

    Rectangle {
        anchors.fill: parent
        color: Theme.ground
    }

    // `documents` is a .rep PROP carrying experiments.list, so this fires
    // whenever the backend pushes a new list.
    Connections {
        target: app.backend
        enabled: app.backend !== null
        function onDocumentsChanged() {
            app.ingestList();
        }
    }

    Component.onCompleted: app.ingestList()

    function ingestList() {
        if (app.backend && app.backend.documents && app.backend.documents !== "{}")
            Core.applyListEnvelope(app.backend.documents);
    }

    /// One call into collect_core. Every screen goes through here, so there is
    /// a single place the call shape is known (SPEC Q12).
    function call(method, request, onData) {
        if (!app.backend) {
            Core.failed("collect_core is not reachable yet");
            return;
        }
        var reply = app.backend.call(method, JSON.stringify(request));
        // The .rep slot is synchronous through QtRO, so the reply is in hand.
        var data = Core.parseEnvelope(reply, method);
        if (data && onData)
            onData(data);
    }

    function openExperiment(experimentId, mine) {
        if (!app.backend) {
            Core.failed("collect_core is not reachable yet");
            return;
        }
        app.openExperimentId = experimentId;
        app.openMine = mine;
        Core.applyExperimentEnvelope(
            app.backend.call("experiments.get", JSON.stringify({ experimentId: experimentId })));
        app.page = "detail";
    }

    function refreshList() {
        app.call("experiments.list", {}, function (data) {
            Core.cards = data.experiments ? data.experiments : [];
        });
    }

    Text {
        anchors.centerIn: parent
        visible: !app.backend
        text: "Connecting to collect_core\u2026"
        font.family: Theme.sans
        font.pixelSize: Theme.body
        color: Theme.ink3
    }

    Loader {
        id: pages
        anchors.fill: parent
        visible: app.backend !== null
        sourceComponent: {
            switch (app.page) {
            case "detail":    return detailPage;
            case "form":      return formPage;
            case "submitted": return submittedPage;
            case "new":       return newPage;
            default:          return homePage;
            }
        }
    }

    Component {
        id: homePage

        Home {
            cards: Core.cards
            onOpenExperiment: function (experimentId, mine) { app.openExperiment(experimentId, mine); }
            onNewExperiment: {
                app.createProblems = ({});
                app.page = "new";
            }
        }
    }

    Component {
        id: detailPage

        Join {
            experiment: Core.experimentDoc
            nowMs: app.nowMs
            mine: app.openMine
            onBack: app.page = "home"
            onOpenProtocol: function (protocolId) {
                app.activeProtocol = Core.protocolById(protocolId);
                if (app.activeProtocol)
                    app.page = "form";
            }
        }
    }

    Component {
        id: formPage

        Form {
            experiment: Core.experimentDoc
            protocol: app.activeProtocol
            accepting: Core.withinWindow(Core.experimentDoc, app.nowMs)
            onSubmitted: function (cid) {
                app.lastCid = cid;
                app.receiptSeen = false;
                app.page = "submitted";
            }
            onCancelled: app.page = "detail"
        }
    }

    Component {
        id: submittedPage

        Submitted {
            protocol: app.activeProtocol
            cid: app.lastCid
            received: app.receiptSeen
            onSubmitAnother: app.page = "form"
            onBackToExperiment: app.page = "detail"
        }
    }

    Component {
        id: newPage

        NewExperiment {
            problems: app.createProblems
            onCancelled: app.page = "home"
            onSave: function (json) {
                var reply = app.backend.call("experiments.create", json);
                var data = Core.parseEnvelope(reply, "experiments.create");
                saving = false;
                if (!data)
                    return;
                if (data.created !== true) {
                    // Per-field problems, keyed for the form to show inline.
                    var byField = ({});
                    for (var i = 0; i < data.problems.length; i++)
                        byField[data.problems[i].field] = data.problems[i].message;
                    app.createProblems = byField;
                    return;
                }
                app.refreshList();
                app.openExperiment(data.experiment.id, true);
            }
        }
    }

    Connections {
        target: Core
        function onReceiptReceived(cid) {
            if (cid === app.lastCid)
                app.receiptSeen = true;
        }
    }
}
