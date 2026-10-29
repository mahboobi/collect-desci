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

    // home | detail | form | submitted | new | edit | builder | results
    property string page: "home"
    property string openExperimentId: ""
    property bool openMine: false
    property var activeProtocol: null
    property string lastCid: ""
    property bool receiptSeen: false
    property var createProblems: ({})
    property var builderProblems: ({})
    /// The protocol being edited, or null when adding a new one.
    property var editingProtocol: null
    /// The results.list payload for the open form.
    property var resultsData: null
    /// The protocol id the open results table belongs to, so it can be
    /// reloaded after a hide without going back to the detail screen.
    property string resultsProtocolId: ""
    /// The results.export payload, once one has been written.
    property var exportResult: null
    /// True while an export or a hide is in flight.
    property bool resultsBusy: false
    /// Warnings collect_core returned about an edit.
    property var editWarnings: []
    /// Whether the open experiment has a CID yet. The head announcement names
    /// one, so there is nothing to announce until it does.
    property bool openPublished: false

    // The typed replica for this module's own backend. Null until the view
    // module is ready, so every read is guarded.
    readonly property var backend: logos.module("collect_ui") // qmllint disable unqualified

    /// Whether a call will actually reach collect_core.
    ///
    /// The replica object exists before the remote behind it is initialized,
    /// so `backend !== null` is not enough: a call made in that window
    /// resolves to nothing, and `JSON.parse` of nothing is
    /// `SyntaxError: Parse error` — which is what the drain timer produced
    /// when it was gated on the replica alone. `coreReady` is the `.rep` PROP
    /// the backend sets once it has talked to collect_core.
    readonly property bool ready: app.backend !== null && app.backend.coreReady === true

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

    Component.onCompleted: {
        // Core has no route to collect_core of its own, so give it one.
        Core.submitter = function (draft, onData) {
            app.call("submissions.submit", {
                experimentId: draft.experiment,
                protocolId: draft.protocol.id,
                values: draft.values
            }, onData);
        };
        app.ingestList();
    }

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
        if (!app.ready) {
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
            app.openPublished = data.published === true;
            app.page = "detail";
        });
    }

    /// Like call(), but the request is already a JSON string.
    function callRaw(method, requestJson, onData, onFail) {
        if (!app.ready) {
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

    /// Open the results table for one form.
    ///
    /// `fresh` clears the last export, which belongs to whichever table was
    /// open when it was written — showing it over a different form's results
    /// would be wrong.
    function openResults(protocolId, fresh) {
        app.call("results.list",
                 { experimentId: app.openExperimentId, protocolId: protocolId },
                 function (data) {
                     app.resultsData = data;
                     app.resultsProtocolId = protocolId;
                     if (fresh)
                         app.exportResult = null;
                     app.page = "results";
                 });
    }

    /// Like call(), for work nobody asked for.
    ///
    /// A timer firing in the background must not take over the error banner:
    /// the banner is for things the person just did, and "the manifest has no
    /// CID yet" on a loop is noise they cannot act on. Failures still go to
    /// the log, so they are not lost — they are just not shouted.
    function callQuietly(method, request, onData) {
        if (!app.ready)
            return;
        logos.watch(app.backend.call(method, JSON.stringify(request)), // qmllint disable unqualified
                    function (reply) {
                        var envelope;
                        try {
                            envelope = JSON.parse(reply);
                        } catch (e) {
                            console.warn("collect:", method, "returned no JSON");
                            return;
                        }
                        if (!envelope.ok) {
                            console.warn("collect:", method,
                                         envelope.error ? envelope.error.message : "failed");
                            return;
                        }
                        if (onData)
                            onData(envelope.data);
                    },
                    function (error) { console.warn("collect:", method, error); });
    }

    /// Hide or show a submitter, then reload the table it changed.
    function setHidden(key, hide) {
        app.resultsBusy = true;
        app.call(hide ? "submitters.hide" : "submitters.show",
                 { experimentId: app.openExperimentId, submitter: key },
                 function () {
                     app.resultsBusy = false;
                     // Reloaded rather than patched in place: hiding
                     // re-publishes the index, and the row counts and the
                     // hidden list both come from it.
                     app.openResults(app.resultsProtocolId, false);
                 });
    }

    function refreshList() {
        app.call("experiments.list", {}, function (data) {
            Core.cards = data.experiments ? data.experiments : [];
        });
    }

    Text {
        anchors.centerIn: parent
        visible: !app.ready
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
            case "edit":      return editPage;
            case "builder":   return builderPage;
            case "results":   return resultsPage;
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
            onAddProtocol: {
                app.editingProtocol = null;
                app.builderProblems = ({});
                app.page = "builder";
            }
            onEditExperiment: {
                app.createProblems = ({});
                app.editWarnings = [];
                app.page = "edit";
            }
            onOpenResults: function (protocolId) {
                app.openResults(protocolId, true);
            }
            onEditProtocol: function (protocolId) {
                app.editingProtocol = Core.protocolById(protocolId);
                app.builderProblems = ({});
                app.page = "builder";
            }
            onOpenProtocol: function (protocolId) {
                app.activeProtocol = Core.protocolById(protocolId);
                if (app.activeProtocol)
                    app.page = "form";
            }
            onFillInProtocol: function (protocolId) {
                // A creator filling in their own form: the same screen a
                // submitter gets, because it is the same act.
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

    Component {
        id: resultsPage

        Results {
            results: app.resultsData
            exportResult: app.exportResult
            busy: app.resultsBusy
            onBack: app.page = "detail"
            onExportRequested: function (format, includeMedia) {
                app.resultsBusy = true;
                app.call("results.export", {
                    experimentId: app.openExperimentId,
                    protocolId: app.resultsProtocolId,
                    format: format,
                    includeMedia: includeMedia
                }, function (data) {
                    app.resultsBusy = false;
                    app.exportResult = data;
                });
            }
            onHideSubmitter: function (key) { app.setHidden(key, true); }
            onShowSubmitter: function (key) { app.setHidden(key, false); }
        }
    }

    Component {
        id: editPage

        NewExperiment {
            id: editScreen
            existing: Core.experimentDoc
            problems: app.createProblems
            warnings: app.editWarnings
            onCancelled: app.page = "detail"
            onSave: function (json) {
                app.callRaw("experiments.update", json, function (data) {
                    editScreen.saving = false;
                    if (data.updated !== true) {
                        var byField = ({});
                        for (var i = 0; i < data.problems.length; i++)
                            byField[data.problems[i].field] = data.problems[i].message;
                        app.createProblems = byField;
                        return;
                    }
                    app.createProblems = ({});
                    app.editWarnings = data.warnings ? data.warnings : [];
                    app.refreshList();
                    // Re-open so the detail screen shows the new version
                    // rather than the one that was edited.
                    app.openExperiment(app.openExperimentId, true);
                }, function () {
                    editScreen.saving = false;
                });
            }
        }
    }

    Component {
        id: builderPage

        Builder {
            id: builderScreen
            experimentId: app.openExperimentId
            existing: app.editingProtocol
            problems: app.builderProblems
            onCancelled: app.page = "detail"
            onSave: function (json) {
                app.callRaw("protocols.saveDraft", json, function (data) {
                    builderScreen.saving = false;
                    if (data.saved !== true) {
                        var byField = ({});
                        for (var i = 0; i < data.problems.length; i++)
                            byField[data.problems[i].field] = data.problems[i].message;
                        app.builderProblems = byField;
                        return;
                    }
                    app.builderProblems = ({});
                    app.refreshList();
                    // Re-open the experiment so the new form appears in its
                    // list and the manifest version the card shows is current.
                    app.openExperiment(app.openExperimentId, true);
                }, function () {
                    builderScreen.saving = false;
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

    // Inbound announcements. collect_core queues them and this drains them,
    // which is what turns an announced submission into an indexed one — so a
    // creator's node makes progress whether or not anyone is on the right
    // screen. 15 seconds: often enough that a submitter does not wait long
    // for a receipt, rare enough to cost nothing when nothing is arriving.
    Timer {
        // `ready`, not `backend !== null`: see the property's note.
        running: app.ready
        interval: 15000
        repeat: true
        triggeredOnStart: true
        onTriggered: app.callQuietly("delivery.drain", {}, function (data) {
            if (data.handled > 0) {
                app.refreshList();
                // A submission may have landed in the table on screen.
                if (app.page === "results" && app.resultsProtocolId !== "")
                    app.openResults(app.resultsProtocolId, false);
            }
            var seen = data.results ? data.results : [];
            for (var i = 0; i < seen.length; i++) {
                var r = seen[i];
                if (r.kind === "receipt" && r.cid === app.lastCid)
                    app.receiptSeen = true;
            }
        });
    }

    // SPEC 3.3: the creator re-publishes `head` every 10 minutes, because
    // Delivery's Store is temporary and a submitter who joins later has no
    // other way to find the newest index.
    Timer {
        // Only once the manifest has an address: a head names the manifest's
        // CID, so announcing one before it has been published would advertise
        // `logos:pending` to everybody listening.
        running: app.ready && app.openMine && app.openPublished
                 && app.openExperimentId !== ""
        interval: 600000
        repeat: true
        triggeredOnStart: true
        onTriggered: app.callQuietly("delivery.publishHead",
                                     { experimentId: app.openExperimentId },
                                     function () {})
    }
}
