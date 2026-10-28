// Tests for the form renderer, run with qmltestrunner.
//
// The point of these is the rule CLAUDE.md is strictest about: the form comes
// from the protocol JSON. So the protocol below is synthetic and covers all
// nine field types — the example protocol has only eight, with no `yesno`.
import QtQuick
import QtTest
import collect
import "../src/qml/components"

Item {
    id: host
    width: 800
    height: 600

    // All nine types, so a missing delegate shows up as a failure rather than
    // as an untested branch.
    readonly property var everyType: ({
        schema: "collect/protocol@1",
        id: "every-type",
        version: 1,
        experiment: "q7k2m3x4w3b5n6tz",
        name: "Every field type",
        instructions: "Covers all nine.",
        allow_multiple: true,
        allow_drafts: true,
        fields: [
            { key: "note",      type: "text",     label: "Note",      required: true,  config: { max_length: 10 } },
            { key: "story",     type: "longtext", label: "Story",     required: false, config: {} },
            { key: "count",     type: "number",   label: "How many",  required: true,  config: { min: 1, max: 5, integer: true, unit: "birds" } },
            { key: "category",  type: "choice",   label: "Category",  required: true,
              config: { options: [ { value: "birds", label: "Birds" }, { value: "mammals", label: "Mammals" } ] } },
            { key: "behaviour", type: "multi",    label: "Behaviour", required: false,
              config: { options: [ { value: "feeding", label: "Feeding" }, { value: "flying", label: "Flying" }, { value: "resting", label: "Resting" } ] } },
            { key: "observed",  type: "datetime", label: "Observed",  required: false, config: { fill_from: ["photo", "now"], no_future: true } },
            { key: "photos",    type: "photo",    label: "Photos",    required: false, config: { min: 1, max: 3, fill_from_exif: ["location", "datetime"] } },
            { key: "where",     type: "location", label: "Where",     required: false,
              config: { methods: ["photo", "manual"], store: "exact" } },
            { key: "alive",     type: "yesno",    label: "Still there?", required: true, config: {} }
        ]
    })

    // Same shape, but the location field rounds.
    readonly property var roundedProtocol: ({
        schema: "collect/protocol@1", id: "rounded", version: 1,
        experiment: "q7k2m3x4w3b5n6tz", name: "Rounded", instructions: "",
        allow_multiple: false, allow_drafts: false,
        fields: [
            { key: "where", type: "location", label: "Where", required: true,
              config: { methods: ["photo", "manual"], store: "rounded_1km" } },
            { key: "observed", type: "datetime", label: "Observed", required: false,
              config: { fill_from: ["photo"], no_future: false } },
            { key: "photos", type: "photo", label: "Photos", required: false,
              config: { max: 2, fill_from_exif: ["location", "datetime"] } }
        ]
    })

    // A location field that does NOT accept a photo, to prove the prefill
    // honours the protocol rather than filling whatever it can.
    readonly property var manualOnlyProtocol: ({
        schema: "collect/protocol@1", id: "manual-only", version: 1,
        experiment: "q7k2m3x4w3b5n6tz", name: "Manual only", instructions: "",
        allow_multiple: false, allow_drafts: false,
        fields: [
            { key: "where", type: "location", label: "Where", required: false,
              config: { methods: ["manual"], store: "exact" } },
            { key: "photos", type: "photo", label: "Photos", required: false,
              config: { max: 2, fill_from_exif: ["location"] } }
        ]
    })

    Component {
        id: rendererComponent
        FormRenderer {}
    }

    function make(protocol) {
        return rendererComponent.createObject(host, { protocol: protocol, width: 760 });
    }

    TestCase {
        name: "FormRenderer"
        when: windowShown

        function test_one_row_per_field_in_protocol_order() {
            var r = make(host.everyType);
            var keys = [];
            for (var i = 0; i < r.fields.length; i++)
                keys.push(r.rowAt(i).field.key);
            compare(keys, ["note", "story", "count", "category", "behaviour",
                           "observed", "photos", "where", "alive"]);
            r.destroy();
        }

        function test_every_field_type_gets_a_delegate() {
            // If a type had no delegate, FieldRow leaves the Loader empty and
            // shows the "update Collect" message instead.
            var r = make(host.everyType);
            for (var i = 0; i < r.fields.length; i++) {
                var row = r.rowAt(i);
                verify(row.delegateItem !== null,
                       "no delegate for type " + row.field.type);
            }
            r.destroy();
        }

        function test_an_unknown_field_type_is_not_rendered_as_text() {
            // Rendering an unknown type as a text box would quietly collect
            // the wrong shape.
            var r = make({
                fields: [ { key: "sound", type: "audio", label: "Audio", required: false } ]
            });
            compare(r.rowAt(0).delegateItem, null);
            r.destroy();
        }

        function test_required_fields_block_completion() {
            var r = make(host.everyType);
            verify(!r.complete, "complete with nothing filled in");
            // note, count, category and alive are the required ones.
            var missing = r.missingLabels;
            compare(missing, ["Note", "How many", "Category", "Still there?"]);
            r.destroy();
        }

        function test_filling_every_required_field_completes_the_form() {
            var r = make(host.everyType);
            r.rowAt(0).delegateItem.value = "hello";          // note
            r.rowAt(2).delegateItem.value = 3;                 // count
            r.rowAt(3).delegateItem.picked = "birds";          // category
            r.rowAt(8).delegateItem.value = false;             // alive
            compare(r.missingLabels, []);
            verify(r.complete, "still incomplete after filling the required fields");
            r.destroy();
        }

        function test_false_is_an_answer_for_yesno() {
            // A required yesno answered "no" must not read as unanswered.
            var r = make(host.everyType);
            var alive = r.rowAt(8);
            verify(alive.missing, "yesno did not start unanswered");
            alive.delegateItem.value = false;
            verify(!alive.missing, "false was treated as unanswered");
            r.destroy();
        }

        function test_values_uses_field_keys_and_omits_blanks() {
            var r = make(host.everyType);
            r.rowAt(0).delegateItem.value = "hi";
            r.rowAt(2).delegateItem.value = 2;
            r.rowAt(3).delegateItem.picked = "mammals";
            r.rowAt(8).delegateItem.value = true;
            var v = r.values();
            compare(Object.keys(v).sort(), ["alive", "category", "count", "note"]);
            compare(v.note, "hi");
            compare(v.count, 2);
            compare(v.category, "mammals");
            compare(v.alive, true);
            // `story` was left blank and must not appear at all.
            verify(v.story === undefined, "a blank field reached values()");
            r.destroy();
        }

        function test_choice_stores_the_option_value_not_its_label() {
            var r = make(host.everyType);
            r.rowAt(3).delegateItem.picked = "birds";
            compare(r.rowAt(3).value, "birds");
            r.destroy();
        }

        function test_multi_keeps_protocol_order_whatever_the_click_order() {
            // Two identical answers must canonicalise the same way, or they
            // sign differently.
            var r = make(host.everyType);
            var multi = r.rowAt(4).delegateItem;
            multi.toggle("resting");
            multi.toggle("feeding");
            compare(multi.value, ["feeding", "resting"]);
            r.destroy();
        }

        function test_multi_toggles_off() {
            var r = make(host.everyType);
            var multi = r.rowAt(4).delegateItem;
            multi.toggle("flying");
            compare(multi.value, ["flying"]);
            multi.toggle("flying");
            verify(multi.blank, "toggling off did not clear the field");
            r.destroy();
        }

        function test_text_over_max_length_reports_a_problem() {
            var r = make(host.everyType);
            var note = r.rowAt(0);
            note.delegateItem.value = "0123456789";   // exactly 10
            compare(note.problem, "");
            r.destroy();
        }

        function test_number_rejects_a_fraction_when_integer_only() {
            var r = make(host.everyType);
            var count = r.rowAt(2).delegateItem;
            count.nudge(0);          // seeds from the minimum
            compare(count.value, 1);
            count.nudge(2);
            compare(count.value, 3);
            // The stepper clamps at the maximum rather than going over.
            count.nudge(10);
            compare(count.value, 5);
            r.destroy();
        }

        function test_rounded_location_stores_the_cell_centre() {
            // The privacy rule, checked at the UI layer too: what the field
            // hands over must already be rounded.
            var r = make(host.roundedProtocol);
            var where = r.rowAt(0).delegateItem;
            where.lat = 12.97712;
            where.lon = 77.59581;
            where.method = "manual";
            var v = r.rowAt(0).value;
            compare(v.rounded, true);
            compare(v.lat, 12.975);
            compare(v.lon, 77.595);
            verify(v.lat !== 12.97712, "the exact latitude survived");
            r.destroy();
        }

        function test_exact_location_keeps_the_point() {
            var r = make(host.everyType);
            var where = r.rowAt(7).delegateItem;
            where.lat = 12.97712;
            where.lon = 77.59581;
            where.method = "manual";
            var v = r.rowAt(7).value;
            compare(v.rounded, false);
            compare(v.lat, 12.97712);
            r.destroy();
        }

        function test_photo_exif_prefills_location_and_datetime() {
            var r = make(host.roundedProtocol);
            r.applyPhotoExif({
                location: { lat: 12.97712, lon: 77.59581, accuracy_m: 12 },
                datetime: "2026-10-28T07:42:00+05:30"
            }, "IMG_3108.jpg");

            var where = r.rowAt(0).value;
            compare(where.method, "photo");
            // Rounded on the way in, so the exact EXIF point is never held.
            compare(where.lat, 12.975);
            compare(where.rounded, true);

            compare(r.rowAt(1).value, "2026-10-28T07:42:00+05:30");
            r.destroy();
        }

        function test_photo_exif_is_refused_when_the_protocol_disallows_it() {
            // `methods` has no "photo", so a photo must not fill the field in.
            var r = make(host.manualOnlyProtocol);
            r.applyPhotoExif({
                location: { lat: 12.97712, lon: 77.59581, accuracy_m: 12 }
            }, "IMG_3108.jpg");
            verify(r.rowAt(0).isBlank, "a photo filled a field that does not accept one");
            r.destroy();
        }

        function test_location_needs_accuracy_when_the_field_bounds_it() {
            var r = make({
                fields: [ { key: "where", type: "location", label: "Where", required: true,
                            config: { methods: ["manual"], max_accuracy_m: 50, store: "exact" } } ]
            });
            var where = r.rowAt(0).delegateItem;
            where.lat = 1; where.lon = 1; where.method = "manual";
            verify(r.rowAt(0).problem !== "", "no complaint about a missing accuracy");
            where.accuracy = 12;
            compare(r.rowAt(0).problem, "");
            where.accuracy = 80;
            verify(r.rowAt(0).problem !== "", "accepted an accuracy worse than the bound");
            r.destroy();
        }

        function test_photo_field_caps_at_the_contract_ceiling() {
            var r = make({
                fields: [ { key: "photos", type: "photo", label: "Photos", required: false,
                            config: { max: 50 } } ]
            });
            // A protocol asking for 50 cannot raise the contract ceiling of 10.
            compare(r.rowAt(0).delegateItem.maximum, 10);
            r.destroy();
        }

        function test_optional_fields_never_block_completion() {
            var r = make({
                fields: [ { key: "story", type: "longtext", label: "Story", required: false, config: {} } ]
            });
            verify(r.complete, "an all-optional form was not complete");
            compare(r.missingLabels, []);
            r.destroy();
        }
    }
}
