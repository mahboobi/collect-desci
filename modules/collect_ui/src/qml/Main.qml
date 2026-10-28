// The submitter flow: S1 -> S2 -> S3.
//
// `view` is the module's entry point in metadata.json. The creator screens
// (C1-C4) are not wired yet.
pragma ComponentBehavior: Bound

import QtQuick
import collect
import "screens"

Item {
    id: app

    // The typed replica for this module's own backend. Null until the view
    // module is ready, which is why every read below is guarded.
    readonly property var backend: logos.module("collect_ui") // qmllint disable unqualified

    Connections {
        target: app.backend
        enabled: app.backend !== null
        // documents is a .rep PROP, so this fires whenever the backend
        // pushes a new manifest — including a later one arriving over
        // Delivery.
        function onDocumentsChanged() {
            if (app.backend.documents && app.backend.documents !== "{}")
                Core.applyOpenEnvelope(app.backend.documents);
        }
    }

    Component.onCompleted: {
        if (app.backend && app.backend.documents && app.backend.documents !== "{}")
            Core.applyOpenEnvelope(app.backend.documents);
    }

    // Lets the harness and a test place themselves inside or outside the
    // experiment window without waiting for a real date.
    property double nowMs: Date.now()

    // Which screen is showing: join, form, submitted.
    property string page: "join"
    property var activeProtocol: null
    property string lastCid: ""
    property bool receiptSeen: false

    anchors.fill: parent

    Rectangle {
        anchors.fill: parent
        color: Theme.ground
    }

    Connections {
        target: Core
        function onReceiptReceived(cid) {
            if (cid === app.lastCid)
                app.receiptSeen = true;
        }
    }

    // Shown until collect_core answers (in either mode), so a slow start does
    // not look like an empty experiment.
    Text {
        anchors.centerIn: parent
        visible: !Core.ready || !Core.experimentDoc
        text: !app.backend
              ? "Connecting to collect_core\u2026"
              : (Core.ready ? "No experiment loaded." : "Loading\u2026")
        font.family: Theme.sans
        font.pixelSize: Theme.body
        color: Theme.ink3
    }

    Loader {
        id: pages
        anchors.fill: parent
        visible: Core.ready && !!Core.experimentDoc
        sourceComponent: {
            switch (app.page) {
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
            nowMs: app.nowMs
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
            onCancelled: app.page = "join"
        }
    }

    Component {
        id: submittedPage

        Submitted {
            protocol: app.activeProtocol
            cid: app.lastCid
            received: app.receiptSeen
            onSubmitAnother: app.page = "form"
            onBackToExperiment: app.page = "join"
        }
    }
}
