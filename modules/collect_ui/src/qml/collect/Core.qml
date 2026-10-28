// The one place the UI talks to collect_core.
//
// CLAUDE.md: build against fixtures first, then wire to live data. Every
// screen calls through here, so switching to live data is a change to this
// file and nothing else.
//
// Which call shape goes in `live` depends on SPEC Q12 (whether collect_ui
// keeps a C++ backend). Both land here:
//   ui-qml-backend -> logos.watch(backend.someSlot(json), onOk, onErr)
//   ui-qml         -> logos.callModuleAsync("collect_core", m, [json], cb)
pragma Singleton
import QtQuick

QtObject {
    id: core

    // COLLECT_FIXTURES=1 serves examples/*.json and never touches a node.
    property bool fixtures: true

    // Set by the host once collect_core answers; screens show a connecting
    // state until then rather than an empty one.
    property bool ready: fixtures

    // ---- the documents the screens read ----
    //
    // Pushed in rather than fetched: the QML sandbox has no filesystem and no
    // network, so nothing here can read examples/*.json. SPEC section 6 puts
    // fixtures behind COLLECT_FIXTURES=1 in collect_core, which means both
    // fixture and live data arrive the same way — through these properties.
    // The harness, which is a plain Qt app and not sandboxed, seeds them
    // directly.
    /// Dashboard cards from experiments.list.
    property var cards: []
    property var experimentDoc: null
    property var protocolDoc: null
    property var submissionDoc: null
    property var indexDoc: null

    signal loaded()
    signal failed(string reason)

    /// Take an `experiments.list` envelope.
    function applyListEnvelope(text) {
        var envelope = core.parseEnvelope(text, "experiments.list");
        if (!envelope)
            return;
        core.cards = envelope.experiments ? envelope.experiments : [];
        core.fixtures = envelope.fixtures === true || core.fixtures;
        core.ready = true;
        core.loaded();
    }

    /// Take an `experiments.get` envelope: one manifest and its protocols.
    function applyExperimentEnvelope(text) {
        var envelope = core.parseEnvelope(text, "experiments.get");
        if (!envelope)
            return;
        core.seed({
            experiment: envelope.experiment,
            protocol: envelope.protocols && envelope.protocols.length > 0 ? envelope.protocols[0] : null
        });
        core.heldProtocols = envelope.protocols ? envelope.protocols : [];
    }

    /// Every protocol document this node holds for the open experiment.
    property var heldProtocols: []

    /// Unwrap an envelope, or report and return null. A failure arrives the
    /// same way as a success, so the caller must not assume.
    function parseEnvelope(text, what) {
        var envelope;
        try {
            envelope = JSON.parse(text);
        } catch (e) {
            core.failed(what + " did not return JSON: " + e);
            return null;
        }
        if (!envelope.ok) {
            core.failed(envelope.error ? envelope.error.message : (what + " failed"));
            return null;
        }
        return envelope.data;
    }

    // Take an `experiments.open` envelope as collect_core returns it.
    //
    // Reads the envelope rather than trusting it: a failure arrives the same
    // way as success, and the screens need to tell the difference.
    function applyOpenEnvelope(text) {
        var envelope;
        try {
            envelope = JSON.parse(text);
        } catch (e) {
            core.failed("experiments.open did not return JSON: " + e);
            return;
        }
        if (!envelope.ok) {
            var detail = envelope.error ? envelope.error.message : "unknown error";
            core.failed(detail);
            return;
        }
        var data = envelope.data;
        core.seed({
            experiment: data.experiment,
            // The screens ask for one protocol at a time; keep them by id.
            protocol: data.protocols && data.protocols.length > 0 ? data.protocols[0] : null
        });
        core.fixtures = data.fixtures === true;
    }

    // Called by the harness, and by the backend once collect_core answers.
    function seed(docs) {
        if (docs.experiment !== undefined)
            core.experimentDoc = docs.experiment;
        if (docs.protocol !== undefined)
            core.protocolDoc = docs.protocol;
        if (docs.submission !== undefined)
            core.submissionDoc = docs.submission;
        if (docs.index !== undefined)
            core.indexDoc = docs.index;
        core.ready = true;
        core.loaded();
    }

    // ---- what the screens ask for ----

    // The protocols of an experiment, as S1 lists them.
    function protocolsOf(experiment) {
        return experiment && experiment.protocols ? experiment.protocols : [];
    }

    function protocolById(id) {
        for (var i = 0; i < core.heldProtocols.length; i++) {
            if (core.heldProtocols[i].id === id)
                return core.heldProtocols[i];
        }
        // Fall back to the single seeded document.
        return core.protocolDoc && core.protocolDoc.id === id ? core.protocolDoc : null;
    }

    // Is the experiment open right now? S1 and S2 both gate on this.
    function withinWindow(experiment, nowMs) {
        if (!experiment)
            return false;
        var from = Date.parse(experiment.starts_at);
        var to = Date.parse(experiment.ends_at);
        return nowMs >= from && nowMs <= to;
    }

    // A short fingerprint of a key, for the "creator name and key
    // fingerprint" S1 shows. Never shows a full key: it is noise to a reader.
    function fingerprint(key) {
        if (!key || key.length < 16)
            return "";
        return key.slice(0, 4) + " " + key.slice(4, 8) + " " + key.slice(8, 12) + " " + key.slice(12, 16);
    }

    function shortCid(cid) {
        if (!cid)
            return "";
        return cid.length > 22 ? cid.slice(0, 16) + "\u2026" + cid.slice(-4) : cid;
    }

    // ---- writes ----
    //
    // In fixtures mode a submit succeeds in memory and the receipt arrives
    // after 3 s, which is what SPEC section 6 specifies for COLLECT_FIXTURES=1.
    signal submitted(string cid)
    signal receiptReceived(string cid)

    function submit(draft, onDone) {
        if (!fixtures) {
            onDone({ ok: false, error: { code: "not_wired", message: "live submit is not wired yet" } });
            return;
        }
        var cid = "logos:zb2rhFixture" + Math.floor(Math.random() * 1e6);
        core.submitted(cid);
        receiptTimer.cid = cid;
        receiptTimer.restart();
        onDone({ ok: true, data: { cid: cid } });
    }

    property Timer receiptTimer: Timer {
        property string cid: ""
        interval: 3000
        onTriggered: core.receiptReceived(cid)
    }
}
