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
import "components"

Item {
    id: app

    // Lets a test place itself inside or outside an experiment window.
    property double nowMs: Date.now()

    // home | detail | form | submitted | new | edit | builder | results
    // | profile | discussion
    property string page: "home"
    /// Where the profile screen's Back goes.
    property string pageBeforeProfile: "home"
    /// profile.get data, and the problems from the last save.
    property var profileData: null
    property var profileProblems: ({})
    property bool profileBusy: false
    property string profileNote: ""
    /// messages.list and members.list for the open experiment.
    property var discussionThread: null
    property var discussionPeople: null
    property bool discussionSending: false
    property string discussionError: ""
    property string openExperimentId: ""
    property bool openMine: false
    property var activeProtocol: null
    property string lastCid: ""
    property bool receiptSeen: false
    // delivery.status's reading for the dashboard's dot. Offline until
    // collect_core answers, which is also the truth before then.
    property var connection: ({ level: "offline", reason: "Connecting to collect_core\u2026" })
    property bool refreshing: false
    // ---- rewards ----
    // The detail page's reward form, and the Pay rewards screen.
    property bool joinRewardBusy: false
    // Joining by link, and this node's submissions to the open experiment.
    property bool joinBusy: false
    property string joinError: ""
    property var mySubmissions: []
    // The Results screen's push to Logos Storage.
    property var pushResult: null
    property bool pushing: false
    property var joinRewardProblems: ({})
    property var rewardPlan: null
    property bool rewardBusy: false
    property var rewardProgress: ({})
    property string rewardFrom: ""

    // The last send failure put in the banner, so a repeat is not.
    property string lastSendFailure: ""
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
    property string openCid: ""
    property var openPublishError: null
    // The open experiment's cover image (a data URI) and its upload state.
    property string openThumbnail: ""
    property bool thumbnailBusy: false
    property string thumbnailError: ""

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

    /// The backend's own image chooser (macOS). See collect_ui_backend.h.
    readonly property bool nativePicker: app.backend !== null && app.backend.nativePicker === true
    // The profile's share switch at the moment a photo was asked for, since
    // the answer arrives later through a signal.
    property bool pickShare: false
    // The photo field waiting for the chooser: {maxMb, room, onEach, onPending}.
    property var formPhotoRequest: null

    anchors.fill: parent

    Rectangle {
        anchors.fill: parent
        color: Theme.ground
    }

    // The notebook the whole module sits on.
    GridPaper {
        anchors.fill: parent
    }

    // Only where the backend has no native chooser (not macOS).
    Loader {
        id: formPhotoPicker
        active: !app.nativePicker
        source: "components/PhotoPicker.qml"
        onLoaded: item.title = "Choose a photo"
    }

    Connections {
        target: formPhotoPicker.item
        ignoreUnknownSignals: true
        function onPicked(url) { app.addFormPhotos([url]); }
    }

    // `documents` is a .rep PROP carrying experiments.list, so this fires
    // whenever the backend pushes a new list.
    Connections {
        target: app.backend
        enabled: app.backend !== null
        function onDocumentsChanged() {
            app.ingestList();
        }
        // The native chooser's answer: one path per line, none on a cancel.
        function onImagePicked(purpose, path) {
            var paths = path.split("\n").map(function (p) { return p.trim(); })
                            .filter(function (p) { return p !== ""; });
            if (purpose === "formPhotos") {
                if (paths.length === 0)
                    app.formPhotoRequest = null;
                else
                    app.addFormPhotos(paths);
                return;
            }
            if (paths.length === 0)
                return;
            if (purpose === "thumbnail")
                app.setThumbnail(paths[0]);
            else if (purpose === "profile")
                app.setProfilePhoto(paths[0], app.pickShare);
        }
    }

    Component.onCompleted: {
        // Core has no route to collect_core of its own, so give it one.
        Core.submitter = function (draft, onData) {
            app.call("submissions.submit", {
                experimentId: draft.experiment,
                protocolId: draft.protocol.id,
                values: draft.values,
                includeWallet: draft.includeWallet === true
            }, onData);
        };
        Core.photoAdder = function (maxMb, room, onEach, onPending) {
            app.formPhotoRequest = { maxMb: maxMb, room: room, onEach: onEach, onPending: onPending };
            if (app.nativePicker)
                app.backend.pickImage("formPhotos", "Choose photos");
            else if (formPhotoPicker.item)
                formPhotoPicker.item.open();
            else
                Core.reportFailure("This copy of Basecamp offers no way to choose a file here.");
        };
        app.ingestList();
    }

    /// Clean and store each picked photo, one call each, and hand every one
    /// that made it to the field that asked. A photo that fails says why in
    /// the banner without losing the others.
    function addFormPhotos(paths) {
        var req = app.formPhotoRequest;
        app.formPhotoRequest = null;
        if (!req)
            return;
        var wanted = paths.slice(0, Math.max(0, req.room));
        if (paths.length > wanted.length)
            Core.reportFailure("Only " + wanted.length + " more photo" + (wanted.length === 1 ? "" : "s")
                               + " fit in this field; the rest were left out.");
        var pending = wanted.length;
        req.onPending(pending);
        wanted.forEach(function (path) {
            app.callEnvelope("media.addPhoto", { path: path, maxMbEach: req.maxMb }, function (e) {
                pending -= 1;
                req.onPending(pending);
                if (e.ok) {
                    req.onEach(e.data);
                    return;
                }
                var name = path.split("/").pop();
                Core.reportFailure(name + " was not added: "
                                   + (e.error && e.error.message ? e.error.message : "it could not be processed"));
            });
        });
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
        if (experimentId !== app.openExperimentId)
            app.thumbnailError = "";
        app.openExperimentId = experimentId;
        app.openMine = mine;
        app.call("experiments.get", { experimentId: experimentId }, function (data) {
            Core.seed({
                experiment: data.experiment,
                protocol: data.protocols && data.protocols.length > 0 ? data.protocols[0] : null
            });
            Core.heldProtocols = data.protocols ? data.protocols : [];
            app.openPublished = data.published === true;
            app.openCid = data.cid ? data.cid : "";
            app.openPublishError = data.publish_error ? data.publish_error : null;
            app.openThumbnail = data.thumbnail ? data.thumbnail : "";
            app.loadMySubmissions();
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

    /// Like call(), but hands back the whole envelope, failures included,
    /// without touching the banner. For screens that put a failure next to
    /// the field it is about.
    function callEnvelope(method, request, onEnvelope) {
        if (!app.ready) {
            onEnvelope({ ok: false, error: { code: "not_wired", message: "collect_core is not reachable yet" } });
            return;
        }
        logos.watch(app.backend.call(method, JSON.stringify(request)), // qmllint disable unqualified
                    function (reply) {
                        var envelope;
                        try {
                            envelope = JSON.parse(reply);
                        } catch (e) {
                            envelope = { ok: false, error: { code: "bad_reply", message: method + " returned no JSON" } };
                        }
                        onEnvelope(envelope);
                    },
                    function (error) {
                        onEnvelope({ ok: false, error: { code: "transport", message: method + ": " + error } });
                    });
    }

    // ---- profile ----

    function openProfile() {
        if (app.page !== "profile")
            app.pageBeforeProfile = app.page;
        app.profileProblems = ({});
        app.profileNote = "";
        app.call("profile.get", {}, function (data) {
            app.profileData = data;
            app.page = "profile";
        });
    }

    function saveProfile(fields) {
        app.profileBusy = true;
        app.callEnvelope("profile.save", { fields: fields }, function (e) {
            app.profileBusy = false;
            if (e.ok) {
                app.profileData = e.data;
                app.profileProblems = ({});
                app.profileNote = "Saved. Shared fields reach your projects' members on the next sync.";
                return;
            }
            app.profileProblems = e.error && e.error.problems ? e.error.problems : ({});
            if (!e.error || !e.error.problems)
                Core.reportFailure(e.error ? e.error.message : "The profile was not saved.");
        });
    }

    function setProfilePhoto(url, share) {
        app.profileBusy = true;
        app.callEnvelope("profile.setPhoto", { path: url, share: share }, function (e) {
            app.profileBusy = false;
            if (e.ok) {
                app.profileData = e.data;
                app.profileProblems = ({});
                return;
            }
            app.profileProblems = { photo: e.error ? e.error.message : "That photo could not be used." };
        });
    }

    // ---- discussion ----

    function openDiscussion() {
        app.discussionError = "";
        app.loadDiscussion(function () { app.page = "discussion"; });
    }

    function loadDiscussion(then) {
        app.call("messages.list", { experimentId: app.openExperimentId }, function (data) {
            app.discussionThread = data;
            app.call("members.list", { experimentId: app.openExperimentId }, function (people) {
                app.discussionPeople = people;
                if (then)
                    then();
            });
        });
    }

    function sendMessage(text, onSent) {
        app.discussionSending = true;
        app.discussionError = "";
        app.callEnvelope("messages.send", { experimentId: app.openExperimentId, text: text }, function (e) {
            app.discussionSending = false;
            if (!e.ok) {
                // Kept on the screen, next to the text that did not go, rather
                // than in the banner: the person is still composing.
                app.discussionError = e.error ? e.error.message : "The message was not sent.";
                return;
            }
            onSent();
            app.loadDiscussion(null);
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
                     if (fresh) {
                         app.exportResult = null;
                         app.pushResult = null;
                     }
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

    function checkConnection() {
        if (!app.ready) {
            app.connection = { level: "offline", reason: "collect_core is not reachable yet." };
            return;
        }
        app.callQuietly("delivery.status", {}, function (data) {
            if (data.connection)
                app.connection = data.connection;
        });
    }

    /// The dashboard's refresh button: anything that arrived, then the list,
    /// then the connection, in that order so the list includes what the
    /// drain just indexed.
    function refreshAll() {
        if (!app.ready) {
            app.checkConnection();
            return;
        }
        app.refreshing = true;
        // Cleared by a timer as well as by the replies: a call that never
        // answers must not leave the button disabled for good.
        refreshGuard.restart();
        app.drainQuietly(function () {
            app.call("experiments.list", {}, function (data) {
                Core.cards = data.experiments ? data.experiments : [];
                app.checkConnection();
                app.refreshing = false;
            });
        });
    }

    Timer {
        id: refreshGuard
        interval: 10000
        onTriggered: app.refreshing = false
    }

    // ---- cover image ----

    /// Set or replace the open experiment's cover. A new manifest version
    /// comes back, so the whole experiment is reloaded rather than patched.
    function setThumbnail(url) {
        app.thumbnailBusy = true;
        app.thumbnailError = "";
        app.callEnvelope("experiments.setThumbnail",
                         { experimentId: app.openExperimentId, path: url },
                         function (e) { app.thumbnailDone(e); });
    }

    function clearThumbnail() {
        app.thumbnailBusy = true;
        app.thumbnailError = "";
        app.callEnvelope("experiments.clearThumbnail", { experimentId: app.openExperimentId },
                         function (e) { app.thumbnailDone(e); });
    }

    function thumbnailDone(e) {
        app.thumbnailBusy = false;
        if (!e.ok) {
            app.thumbnailError = e.error && e.error.message ? e.error.message
                                                             : "The cover image was not changed.";
            return;
        }
        app.openThumbnail = e.data.thumbnail ? e.data.thumbnail : "";
        app.openExperiment(app.openExperimentId, app.openMine);
        app.refreshList();
    }

    // ---- joining and the outbox ----

    function joinLink(link) {
        app.joinBusy = true;
        app.joinError = "";
        app.callEnvelope("experiments.open", { link: link }, function (e) {
            app.joinBusy = false;
            if (!e.ok) {
                app.joinError = e.error && e.error.message ? e.error.message : "That link could not be opened.";
                return;
            }
            app.refreshList();
            var missing = e.data.missing_forms ? e.data.missing_forms : [];
            if (missing.length > 0)
                Core.reportFailure(missing.length + (missing.length === 1 ? " form" : " forms")
                                   + " could not be fetched yet; open the link again later to try.");
            app.openExperiment(e.data.experiment.id, false);
        });
    }

    function loadMySubmissions() {
        if (app.openMine || app.openExperimentId === "") {
            app.mySubmissions = [];
            return;
        }
        app.callQuietly("submissions.mine", { experimentId: app.openExperimentId }, function (d) {
            app.mySubmissions = d.submissions ? d.submissions : [];
        });
    }

    // ---- push results to Logos Storage ----

    function pushResults() {
        app.pushing = true;
        app.pushResult = null;
        app.callEnvelope("results.publish",
                         { experimentId: app.openExperimentId, protocolId: app.resultsProtocolId },
                         function (e) {
                             app.pushing = false;
                             app.pushResult = e;
                             if (!e.ok)
                                 return;
                             // A new manifest version lists the snapshot;
                             // reload it quietly so "last published" follows.
                             app.callQuietly("experiments.get", { experimentId: app.openExperimentId },
                                             function (d) { Core.experimentDoc = d.experiment; });
                             app.refreshList();
                         });
    }

    // ---- rewards ----

    function setReward(amount, maxPerSubmitter) {
        app.joinRewardBusy = true;
        app.joinRewardProblems = ({});
        app.callEnvelope("experiments.setReward",
                         { experimentId: app.openExperimentId, amount: amount, maxPerSubmitter: maxPerSubmitter },
                         function (e) {
                             app.joinRewardBusy = false;
                             if (e.ok) {
                                 app.openExperiment(app.openExperimentId, app.openMine);
                                 app.refreshList();
                                 return;
                             }
                             if (e.error && e.error.problems)
                                 app.joinRewardProblems = e.error.problems;
                             else
                                 Core.reportFailure(e.error ? e.error.message : "The reward was not changed.");
                         });
    }

    function setApproval(cid, status) {
        app.resultsBusy = true;
        app.callEnvelope("results.setApproval",
                         { experimentId: app.openExperimentId, cid: cid, status: status },
                         function (e) {
                             app.resultsBusy = false;
                             if (!e.ok)
                                 Core.reportFailure(e.error ? e.error.message : "That review was not saved.");
                             app.openResults(app.resultsProtocolId, false);
                         });
    }

    /// The paying account defaults to the wallet in the creator's profile,
    /// which is usually their own LEZ address.
    function openRewards() {
        if (app.rewardFrom === "" && app.profileData && app.profileData.fields
                && app.profileData.fields.wallet && app.profileData.fields.wallet.value)
            app.rewardFrom = app.profileData.fields.wallet.value;
        app.rewardProgress = ({});
        app.loadRewardPlan(function () { app.page = "rewards"; });
    }

    function loadRewardPlan(then) {
        app.rewardBusy = true;
        app.callEnvelope("rewards.plan", { experimentId: app.openExperimentId, from: app.rewardFrom },
                         function (e) {
                             app.rewardBusy = false;
                             if (!e.ok) {
                                 Core.reportFailure(e.error ? e.error.message : "The payout plan could not be made.");
                                 return;
                             }
                             app.rewardPlan = e.data;
                             if (then)
                                 then();
                         });
    }

    /// One payment per call, one after another, each exactly as shown. A
    /// refusal stops the run: whatever caused it — the plan changed, the
    /// wallet closed — is worth looking at before paying anyone else.
    function payRewards(from) {
        if (!app.rewardPlan || app.rewardBusy)
            return;
        var queue = app.rewardPlan.payees.slice();
        app.rewardBusy = true;
        var mark = function (key, state) {
            var next = Object.assign({}, app.rewardProgress);
            next[key] = state;
            app.rewardProgress = next;
        };
        var step = function () {
            if (queue.length === 0) {
                app.rewardBusy = false;
                app.loadRewardPlan(null);
                return;
            }
            var p = queue.shift();
            mark(p.submitter, "paying");
            app.callEnvelope("rewards.pay", {
                experimentId: app.openExperimentId, from: from, submitter: p.submitter,
                cids: p.submissions, amountLepta: p.amount_lepta
            }, function (e) {
                if (e.ok) {
                    mark(p.submitter, "paid");
                    step();
                    return;
                }
                mark(p.submitter, "failed: " + (e.error ? e.error.message : "refused"));
                app.rewardBusy = false;
                app.loadRewardPlan(null);
            });
        };
        step();
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
            case "profile":   return profilePage;
            case "discussion": return discussionPage;
            case "rewards":   return rewardsPage;
            default:          return homePage;
            }
        }
    }

    Component {
        id: homePage

        Home {
            cards: Core.cards
            joinBusy: app.joinBusy
            joinError: app.joinError
            onJoinLink: function (link) { app.joinLink(link); }
            connection: app.connection
            refreshing: app.refreshing
            onRefresh: app.refreshAll()
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
            published: app.openPublished
            cid: app.openCid
            publishError: app.openPublishError
            thumbnail: app.openThumbnail
            thumbnailBusy: app.thumbnailBusy
            thumbnailError: app.thumbnailError
            onPickThumbnail: function (url) { app.setThumbnail(url); }
            rewardBusy: app.joinRewardBusy
            rewardProblems: app.joinRewardProblems
            onSetReward: function (amount, maxPerSubmitter) { app.setReward(amount, maxPerSubmitter); }
            onOpenRewards: app.openRewards()
            mySubmissions: app.mySubmissions
            nativePicker: app.nativePicker
            onRequestThumbnailPick: app.backend.pickImage("thumbnail", "Choose a cover image")
            onClearThumbnail: app.clearThumbnail()
            onPublishNow: {
                app.call("experiments.publish", { experimentId: app.openExperimentId },
                         function () { app.openExperiment(app.openExperimentId, true); });
            }
            onBack: app.page = "home"
            onOpenDiscussion: app.openDiscussion()
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
        id: profilePage

        Profile {
            profile: app.profileData
            problems: app.profileProblems
            busy: app.profileBusy
            savedNote: app.profileNote
            onBack: app.page = app.pageBeforeProfile
            onSave: function (fields) { app.saveProfile(fields); }
            onPickPhoto: function (url, share) { app.setProfilePhoto(url, share); }
            nativePicker: app.nativePicker
            onRequestPhotoPick: function (share) {
                app.pickShare = share;
                app.backend.pickImage("profile", "Choose a profile photo");
            }
            onClearPhoto: {
                app.call("profile.clearPhoto", {}, function (data) { app.profileData = data; });
            }
        }
    }

    Component {
        id: discussionPage

        Discussion {
            id: discussion
            experiment: Core.experimentDoc
            thread: app.discussionThread
            people: app.discussionPeople
            sending: app.discussionSending
            sendError: app.discussionError
            onBack: app.page = "detail"
            onSend: function (text) { app.sendMessage(text, function () { discussion.clearComposer(); }); }
        }
    }

    // The profile, from anywhere: top right, above every page.
    Button {
        id: profileButton
        anchors.top: errorBanner.bottom
        anchors.right: parent.right
        anchors.topMargin: Theme.s16
        anchors.rightMargin: Theme.s20
        z: 5
        visible: app.ready && app.page !== "profile"
        width: profileLabel.implicitWidth + Theme.touchTarget + Theme.s12
        height: Theme.touchTarget
        Accessible.name: "Open your science profile"
        onClicked: app.openProfile()
        contentItem: Row {
            spacing: Theme.s8
            Text {
                id: profileLabel
                anchors.verticalCenter: parent.verticalCenter
                text: "Profile"
                font.family: Theme.sans
                font.pixelSize: Theme.small
                font.weight: Theme.medium
                color: Theme.ink2
            }
            Avatar {
                anchors.verticalCenter: parent.verticalCenter
                size: 36
                photo: app.profileData && app.profileData.fields && app.profileData.fields.photo.value
                       ? app.profileData.fields.photo.value : ""
                name: app.profileData && app.profileData.fields && app.profileData.fields.alias.value
                      ? app.profileData.fields.alias.value : ""
                fallback: "☺"
            }
        }
        background: Rectangle {
            radius: Theme.rPill
            color: profileButton.down ? Theme.surfaceSunken : Theme.surface
            border.width: 1
            border.color: Theme.line
        }
    }

    // Load the profile once, so the button shows the person's photo, and
    // read the connection straight away rather than at the first timer tick.
    // The only `onReadyChanged` in this file: a second one is a compile
    // error that stops the whole app opening.
    onReadyChanged: {
        app.checkConnection();
        if (app.ready && app.profileData === null)
            app.callQuietly("profile.get", {}, function (data) { app.profileData = data; });
    }

    Component {
        id: formPage

        Form {
            experiment: Core.experimentDoc
            protocol: app.activeProtocol
            wallet: app.profileData && app.profileData.fields && app.profileData.fields.wallet
                    && app.profileData.fields.wallet.value ? app.profileData.fields.wallet.value : ""
            accepting: Core.withinWindow(Core.experimentDoc, app.nowMs)
            onSubmitted: function (cid) {
                app.lastCid = cid;
                app.receiptSeen = false;
                app.loadMySubmissions();
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
            rewarding: Core.rewardOf(Core.experimentDoc) !== null
            onSetApproval: function (cid, status) { app.setApproval(cid, status); }
            published: {
                var list = Core.experimentDoc && Core.experimentDoc.results ? Core.experimentDoc.results : [];
                for (var i = 0; i < list.length; i++)
                    if (list[i].protocol === app.resultsProtocolId)
                        return list[i];
                return null;
            }
            pushResult: app.pushResult
            pushing: app.pushing
            onPushToStorage: app.pushResults()
        }
    }

    Component {
        id: rewardsPage

        Rewards {
            experiment: Core.experimentDoc
            plan: app.rewardPlan
            busy: app.rewardBusy
            progress: app.rewardProgress
            fromAddress: app.rewardFrom
            onBack: app.page = "detail"
            onCheckFrom: function (address) {
                app.rewardFrom = address;
                app.loadRewardPlan(null);
            }
            onPayAll: function (address) { app.payRewards(address); }
            onRefresh: {
                app.rewardBusy = true;
                app.callEnvelope("rewards.refresh", { experimentId: app.openExperimentId }, function () {
                    app.rewardBusy = false;
                    app.loadRewardPlan(null);
                });
            }
            onResolve: function (payoutId, outcome) {
                app.callEnvelope("rewards.resolve",
                                 { experimentId: app.openExperimentId, payoutId: payoutId, outcome: outcome },
                                 function (e) {
                                     if (!e.ok)
                                         Core.reportFailure(e.error ? e.error.message : "Not resolved.");
                                     app.loadRewardPlan(null);
                                 });
            }
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
        // Faster while a discussion is open, so a reply shows in seconds.
        interval: app.page === "discussion" ? 4000 : 15000
        repeat: true
        triggeredOnStart: true
        onTriggered: {
            app.checkConnection();
            app.drainQuietly();
        }
    }

    /// Collect what Delivery has brought in, then `then()` if given.
    function drainQuietly(then) {
        app.callQuietly("delivery.drain", {}, function (data) {
            // A cover image arrived for an experiment on screen.
            if (data.thumbnails_fetched > 0) {
                if (data.handled === 0)
                    app.refreshList();
                if (app.page === "detail" && app.openExperimentId !== "")
                    app.callQuietly("experiments.get", { experimentId: app.openExperimentId },
                                    function (d) { app.openThumbnail = d.thumbnail ? d.thumbnail : ""; });
            }
            if (data.handled > 0) {
                app.refreshList();
                // A submission may have landed in the table on screen.
                if (app.page === "results" && app.resultsProtocolId !== "")
                    app.openResults(app.resultsProtocolId, false);
            }
            var seen = data.results ? data.results : [];
            var talk = false;
            for (var i = 0; i < seen.length; i++) {
                var r = seen[i];
                if ((r.kind === "message" || r.kind === "profile") && r.experiment === app.openExperimentId)
                    talk = true;
                if (r.kind === "receipt" && r.cid === app.lastCid)
                    app.receiptSeen = true;
                // A send that the network refused is about something the
                // person did, so it goes to the banner — unlike the rest of
                // this timer's work. Once per kind of failure: the creator's
                // head is re-sent every ten minutes, and the same banner
                // each time says nothing new. The dot keeps the live state.
                if (r.reason === "send_failed" && r.detail !== app.lastSendFailure) {
                    app.lastSendFailure = r.detail;
                    Core.reportFailure("Your data is saved on Logos Storage, but the network did not carry "
                                       + "the announcement (" + r.detail + "). On the testnet this usually "
                                       + "means this node has no RLN membership yet; register one in the "
                                       + "RLN Membership app. Collect keeps trying.");
                }
            }
            if (talk && app.page === "discussion")
                app.loadDiscussion(null);
            var outboxMoved = data.resent > 0 || seen.some(function (r) { return r.kind === "receipt"; });
            if (outboxMoved && app.page === "detail")
                app.loadMySubmissions();
            if (then)
                then();
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
