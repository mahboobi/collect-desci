// The whole app: dashboard, experiment detail, the submitter flow, and
// creating an experiment.
//
// Routing lives here so each screen stays a view over Core and emits signals
// rather than navigating itself.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
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
    ///
    /// A .rep SLOT with a return value is **asynchronous** over QtRO: calling
    /// it hands back a pending call, not the value. logos.watch resolves it.
    /// Treating the return as a string is the bug that made every call fail
    /// silently.
    function call(method, request, onData) {
        if (!app.backend) {
            Core.reportFailure("collect_core is not reachable yet");
            return;
        }
        logos.watch(app.backend.call(method, JSON.stringify(request)), // qmllint disable unqualified
                    function (reply) {
                        var data = Core.parseEnvelope(reply, method);
                        if (data && onData)
                            onData(data);
                    },
                    function (error) {
                        Core.reportFailure(method + ": " + error);
                    });
    }

    function openExperiment(experimentId, mine) {
        app.openExperimentId = experimentId;
        app.openMine = mine;
        app.call("experiments.get", { experimentId: experimentId }, function (data) {
            Core.seed({
                experiment: data.experiment,
                protocol: data.protocols && data.protocols.length > 0 ? data.protocols[0] : null
            });
            Core.heldProtocols = data.protocols ? data.protocols : [];
            app.page = "detail";
        });
    }

    /// Like call(), but the request is already a JSON string.
    function callRaw(method, requestJson, onData, onFail) {
        if (!app.backend) {
            Core.reportFailure("collect_core is not reachable yet");
            if (onFail)
                onFail();
            return;
        }
        logos.watch(app.backend.call(method, requestJson), // qmllint disable unqualified
                    function (reply) {
                        var data = Core.parseEnvelope(reply, method);
                        if (!data) {
                            if (onFail)
                                onFail();
                            return;
                        }
                        if (onData)
                            onData(data);
                    },
                    function (error) {
                        Core.reportFailure(method + ": " + error);
                        if (onFail)
                            onFail();
                    });
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

    // Failures were silent before this: Core.failed had no listener.
    Rectangle {
        id: errorBanner
        visible: Core.lastError !== ""
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        height: visible ? bannerText.implicitHeight + Theme.s24 : 0
        z: 10
        color: Theme.amberTint

        Text {
            id: bannerText
            anchors.verticalCenter: parent.verticalCenter
            anchors.left: parent.left
            anchors.right: dismiss.left
            anchors.leftMargin: Theme.s20
            anchors.rightMargin: Theme.s12
            wrapMode: Text.Wrap
            text: Core.lastError
            font.family: Theme.sans
            font.pixelSize: Theme.small
            color: Theme.amber
        }

        Button {
            id: dismiss
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            anchors.rightMargin: Theme.s12
            width: Theme.touchTarget
            height: Theme.touchTarget
            text: "\u2715"
            Accessible.name: "Dismiss this message"
            onClicked: Core.lastError = ""
            contentItem: Text {
                text: dismiss.text
                font.pixelSize: Theme.body
                color: Theme.amber
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
            }
            background: null
        }
    }

    Loader {
        id: pages
        anchors.top: errorBanner.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
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
            id: newScreen
            onSave: function (json) {
                app.callRaw("experiments.create", json, function (data) {
                    newScreen.saving = false;
                    if (data.created !== true) {
                        // Per-field problems, keyed for the form to show
                        // inline.
                        var byField = ({});
                        for (var i = 0; i < data.problems.length; i++)
                            byField[data.problems[i].field] = data.problems[i].message;
                        app.createProblems = byField;
                        return;
                    }
                    app.createProblems = ({});
                    app.refreshList();
                    app.openExperiment(data.experiment.id, true);
                }, function () {
                    newScreen.saving = false;
                });
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
