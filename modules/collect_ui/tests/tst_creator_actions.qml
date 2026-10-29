// The creator's three new actions, at the screen boundary.
//
// What these check is the plumbing, not collect_core's rules: that the
// buttons emit what the router expects, that prefilling an edit reaches the
// fields, and that an edit's payload carries the id. The rules themselves are
// tested in Rust, and duplicating them here would only let the two drift.
import QtQuick
import QtTest
import collect
import "../src/qml/screens"

Item {
    id: host
    width: 1000
    height: 900

    Component {
        id: resultsComponent
        Results {}
    }

    Component {
        id: detailsComponent
        NewExperiment {}
    }

    /// One row, as results.list sends it.
    readonly property var sampleResults: ({
        protocol: { id: "bird-sighting", name: "Bird sighting" },
        columns: [ { key: "count", label: "How many", type: "number", options: null } ],
        rows: [ {
            cid: "logos:zAbc",
            submitter: "a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4",
            submitter_name: "Priya",
            version: 1,
            submitted_at: "2026-10-25T09:00:00Z",
            received_at: "2026-10-25T09:00:05Z",
            values: { count: 3 }
        } ],
        hidden_keys: [],
        summary: { shown: 1, submitters: 1, indexed: 1, hidden: 0, not_fetched: 0 }
    })

    readonly property var hiddenResults: ({
        protocol: { id: "bird-sighting", name: "Bird sighting" },
        columns: [],
        rows: [],
        hidden_keys: [ "b2c3d4e5b2c3d4e5b2c3d4e5b2c3d4e5b2c3d4e5b2c3d4e5b2c3d4e5b2c3d4e5" ],
        summary: { shown: 0, submitters: 0, indexed: 1, hidden: 1, not_fetched: 0 }
    })

    TestCase {
        name: "CreatorActions"
        when: windowShown

        function test_export_asks_for_the_format_that_was_pressed() {
            var screen = resultsComponent.createObject(host, { results: host.sampleResults });
            var asked = [];
            screen.exportRequested.connect(function (format, includeMedia) {
                asked.push({ format: format, media: includeMedia });
            });

            // Both buttons live in one Repeater, so they are found by text
            // rather than by id.
            var csv = host.buttonSaying(screen, "Export CSV");
            verify(csv !== null, "the CSV button exists");
            csv.clicked();
            compare(asked.length, 1);
            compare(asked[0].format, "csv");
            // The photos box is off by default: fetching every image is slow
            // and should be asked for.
            compare(asked[0].media, false);

            var js = host.buttonSaying(screen, "Export JSON");
            verify(js !== null, "the JSON button exists");
            js.clicked();
            compare(asked[1].format, "json");
            screen.destroy();
        }

        function test_export_buttons_are_disabled_with_no_rows() {
            // Writing an empty CSV is not useful and looks like a failure.
            var screen = resultsComponent.createObject(host, { results: host.hiddenResults });
            var csv = host.buttonSaying(screen, "Export CSV");
            verify(csv !== null);
            compare(csv.enabled, false);
            screen.destroy();
        }

        function test_export_result_shows_the_path_and_the_media_count() {
            var screen = resultsComponent.createObject(host, {
                results: host.sampleResults,
                exportResult: {
                    format: "csv",
                    filename: "collect-x-y-20261029.csv",
                    path: "/tmp/exports/collect-x-y-20261029.csv",
                    bytes: 120,
                    rows: 1,
                    columns: 1,
                    media: { requested: true, wanted: 2, written: 1, failed: [ { cid: "logos:zNo" } ] }
                }
            });
            // The path has to be visible in full: the QML side cannot open a
            // file manager, so it is the whole deliverable.
            var shown = host.textContaining(screen, "/tmp/exports/collect-x-y-20261029.csv");
            verify(shown !== null, "the path is on screen");
            var media = host.textContaining(screen, "1 of 2 photo(s) fetched");
            verify(media !== null, "the media count is on screen");
            screen.destroy();
        }

        function test_hide_passes_the_full_key_not_the_fingerprint() {
            // The fingerprint is what is displayed; hiding the fingerprint
            // would hide nobody.
            var screen = resultsComponent.createObject(host, { results: host.sampleResults });
            var asked = [];
            screen.hideSubmitter.connect(function (key) { asked.push(key); });
            var hide = host.buttonSaying(screen, "Hide");
            verify(hide !== null, "the hide button exists");
            hide.clicked();
            compare(asked.length, 1);
            compare(asked[0], host.sampleResults.rows[0].submitter);
            screen.destroy();
        }

        function test_a_hidden_submitter_can_be_shown_again() {
            // Without this the creator can hide someone and then has no way
            // to find them again.
            var screen = resultsComponent.createObject(host, { results: host.hiddenResults });
            var asked = [];
            screen.showSubmitter.connect(function (key) { asked.push(key); });
            var show = host.buttonSaying(screen, "Show again");
            verify(show !== null, "the show button exists");
            show.clicked();
            compare(asked[0], host.hiddenResults.hidden_keys[0]);
            screen.destroy();
        }

        function test_buttons_are_disabled_while_an_action_is_in_flight() {
            var screen = resultsComponent.createObject(host, {
                results: host.sampleResults,
                busy: true
            });
            compare(host.buttonSaying(screen, "Hide").enabled, false);
            compare(host.buttonSaying(screen, "Export CSV").enabled, false);
            screen.destroy();
        }

        function test_a_new_experiment_screen_starts_empty() {
            var screen = detailsComponent.createObject(host, {});
            compare(screen.editing, false);
            verify(host.textContaining(screen, "New experiment") !== null);
            verify(host.buttonSaying(screen, "Create experiment") !== null);
            screen.destroy();
        }

        function test_editing_prefills_every_field_from_the_experiment() {
            var screen = detailsComponent.createObject(host, {
                existing: {
                    id: "q7k2m3x4w3b5n6tz",
                    title: "Urban birds",
                    description: "Counting birds.",
                    starts_at: "2026-10-20T00:00:00+05:30",
                    ends_at: "2026-11-20T23:59:59+05:30",
                    timezone: "Asia/Kolkata",
                    creator: { key: "a1", name: "Dr Rao" }
                }
            });
            compare(screen.editing, true);
            verify(host.textContaining(screen, "Edit experiment") !== null);
            // Prefilled, and still editable — a binding to `existing` would
            // have made every keystroke revert.
            verify(host.textContaining(screen, "Urban birds") !== null,
                   "the title reached the field");
            verify(host.textContaining(screen, "Asia/Kolkata") !== null,
                   "the time zone reached the field");
            screen.destroy();
        }

        function test_an_edit_payload_carries_the_id_and_the_creator_name() {
            // Without the id collect_core cannot tell which experiment this
            // is; without the name the display name would be cleared by every
            // edit.
            var screen = detailsComponent.createObject(host, {
                existing: {
                    id: "q7k2m3x4w3b5n6tz",
                    title: "Urban birds",
                    description: "",
                    starts_at: "2026-10-20T00:00:00+05:30",
                    ends_at: "2026-11-20T23:59:59+05:30",
                    timezone: "UTC",
                    creator: { key: "a1", name: "Dr Rao" }
                }
            });
            var sent = null;
            screen.save.connect(function (json) { sent = JSON.parse(json); });
            var save = host.buttonSaying(screen, "Save changes");
            verify(save !== null, "the save button says it is an edit");
            verify(save.enabled, "a prefilled form is complete");
            save.clicked();
            verify(sent !== null, "the payload was emitted");
            compare(sent.experimentId, "q7k2m3x4w3b5n6tz");
            compare(sent.creatorName, "Dr Rao");
            compare(sent.title, "Urban birds");
            verify(sent.startsAt.indexOf("2026-10-20") === 0, sent.startsAt);
            screen.destroy();
        }

        function test_an_edit_shows_a_warning_about_submissions_it_excludes() {
            var screen = detailsComponent.createObject(host, {
                existing: { id: "q7k2m3x4w3b5n6tz", title: "x", description: "",
                            starts_at: "2026-10-20T00:00:00Z", ends_at: "2026-11-20T00:00:00Z",
                            timezone: "UTC", creator: { key: "a1", name: "" } },
                warnings: [ { code: "submissions_outside_window",
                              message: "1 submission(s) already accepted fall outside the new dates." } ]
            });
            verify(host.textContaining(screen, "fall outside the new dates") !== null);
            screen.destroy();
        }
    }

    // ---- finding things on a screen ----
    //
    // By visible text, because the controls these tests care about are built
    // by Repeaters and have no stable ids to reach.

    function buttonSaying(root, label) {
        return host.find(root, function (item) {
            return item.hasOwnProperty("clicked") && item.hasOwnProperty("text")
                   && String(item.text) === label;
        });
    }

    function textContaining(root, fragment) {
        return host.find(root, function (item) {
            return item.hasOwnProperty("text") && !item.hasOwnProperty("clicked")
                   && String(item.text).indexOf(fragment) !== -1;
        });
    }

    function find(item, matches) {
        if (!item)
            return null;
        if (matches(item))
            return item;
        var children = item.children ? item.children : [];
        for (var i = 0; i < children.length; i++) {
            var hit = host.find(children[i], matches);
            if (hit)
                return hit;
        }
        // Flickables keep their content under contentItem, which is not in
        // `children`.
        if (item.contentItem && item.contentItem !== item) {
            var inner = host.find(item.contentItem, matches);
            if (inner)
                return inner;
        }
        return null;
    }
}
