// Saving a form, driven the way a person drives it.
//
// The builder had no test over its save path at all, and "forms are not
// saving, no error" is exactly the failure that gap allows: every piece works
// and the payload never leaves.
import QtQuick
import QtTest
import collect
import "../src/qml/screens"

Item {
    id: host
    width: 1100
    height: 1000

    Component {
        id: builderComponent
        Builder {}
    }

    TestCase {
        name: "BuilderSave"
        when: windowShown

        function make() {
            return builderComponent.createObject(host, { experimentId: "q7k2m3x4w3b5n6tz" });
        }

        function test_save_is_refused_until_there_is_a_name_and_a_field() {
            // Not an error state, a disabled button — but if `complete` were
            // wrong, Save would never enable and nothing would happen when
            // pressed, with nothing shown.
            var b = make();
            compare(b.complete, false, "nothing filled in");

            b.addField("text");
            compare(b.fields.length, 1);
            compare(b.complete, false, "a field but no name");

            verify(host.setName(b, "Bird sighting") !== null, "the name field was found");
            compare(b.complete, true, "a name and a field");
            b.destroy();
        }

        function test_saving_emits_the_whole_draft() {
            var b = make();
            b.addField("text");
            b.addField("number");
            verify(host.setName(b, "Bird sighting") !== null, "the name field was found");

            var sent = null;
            b.save.connect(function (json) { sent = JSON.parse(json); });
            var save = host.buttonSaying(b, "Save form");
            verify(save !== null, "the save button exists");
            verify(save.enabled, "a complete form can be saved");
            save.clicked();

            verify(sent !== null, "the draft was emitted");
            compare(sent.experimentId, "q7k2m3x4w3b5n6tz");
            compare(sent.protocol.name, "Bird sighting");
            compare(sent.protocol.fields.length, 2);
            // Field order is the form's order, which is what the renderer and
            // the results columns both follow.
            compare(sent.protocol.fields[0].type, "text");
            compare(sent.protocol.fields[1].type, "number");
            // Every field needs a key: the submission is stored under it.
            for (var i = 0; i < sent.protocol.fields.length; i++)
                verify(String(sent.protocol.fields[i].key).length > 0,
                       "field " + i + " has a key");
            b.destroy();
        }

        function test_two_fields_of_one_type_do_not_collide_on_a_key() {
            // Values are keyed by field key, so a duplicate silently loses an
            // answer.
            var b = make();
            b.addField("text");
            b.addField("text");
            b.addField("text");
            host.setName(b, "Notes");
            var keys = [];
            for (var i = 0; i < b.fields.length; i++)
                keys.push(b.fields[i].key);
            for (var j = 0; j < keys.length; j++)
                for (var k = j + 1; k < keys.length; k++)
                    verify(keys[j] !== keys[k], "keys " + j + " and " + k + " collide: " + keys[j]);
            b.destroy();
        }

        function test_the_button_says_it_is_working_once_pressed() {
            // `saving` is cleared by the router when the reply lands. If it
            // were never set the person gets no feedback at all, which reads
            // as "nothing happened".
            var b = make();
            b.addField("text");
            host.setName(b, "Bird sighting");
            var save = host.buttonSaying(b, "Save form");
            save.clicked();
            compare(b.saving, true);
            verify(host.buttonSaying(b, "Saving…") !== null,
                   "the button says it is saving");
            b.destroy();
        }

        function test_removing_the_last_field_disables_save_again() {
            var b = make();
            b.addField("text");
            host.setName(b, "Bird sighting");
            compare(b.complete, true);
            b.removeAt(0);
            compare(b.fields.length, 0);
            compare(b.complete, false);
            b.destroy();
        }

        function test_an_existing_form_keeps_its_id_so_it_is_a_new_version() {
            // Without the id collect_core treats the save as a new form and
            // the old one stays in the manifest.
            var b = builderComponent.createObject(host, {
                experimentId: "q7k2m3x4w3b5n6tz",
                existing: {
                    id: "wildlife-sighting",
                    name: "Wildlife sighting",
                    instructions: "",
                    fields: [ { key: "count", type: "number", label: "How many", required: true } ]
                }
            });
            var sent = null;
            b.save.connect(function (json) { sent = JSON.parse(json); });
            var save = host.buttonSaying(b, "Save form");
            verify(save !== null && save.enabled, "a prefilled form is complete");
            save.clicked();
            verify(sent !== null, "the draft was emitted");
            compare(sent.protocol.id, "wildlife-sighting");
            b.destroy();
        }
    }

    // ---- driving the screen ----

    /// `verify` only exists inside a TestCase, so a helper returns instead
    /// and the test body asserts.
    function setName(builder, text) {
        var field = host.find(builder, function (item) {
            return item.hasOwnProperty("text") && item.hasOwnProperty("cursorPosition")
                   && item.hasOwnProperty("readOnly") && !item.readOnly;
        });
        if (field)
            field.text = text;
        return field;
    }

    function buttonSaying(root, label) {
        return host.find(root, function (item) {
            return item.hasOwnProperty("clicked") && item.hasOwnProperty("text")
                   && String(item.text) === label;
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
        if (item.contentItem && item.contentItem !== item) {
            var inner = host.find(item.contentItem, matches);
            if (inner)
                return inner;
        }
        return null;
    }
}
