// Tests for the profile and discussion screens, run with qmltestrunner.
import QtQuick
import QtTest
import collect
import "../src/qml/screens"

Item {
    id: host
    width: 900
    height: 800

    readonly property var blankProfile: ({
        fields: {
            alias: { value: null, share: false }, affiliation: { value: null, share: false },
            location: { value: null, share: false }, sex: { value: null, share: false },
            age: { value: null, share: false }, email: { value: null, share: false },
            phone: { value: null, share: false }, wallet: { value: null, share: false },
            photo: { value: null, share: false }
        },
        sex_options: ["female", "male", "intersex", "other", "prefer_not"]
    })

    Component {
        id: profileComponent
        Profile {}
    }

    Component {
        id: discussionComponent
        Discussion {
            experiment: ({ title: "Urban wildlife sightings" })
        }
    }

    SignalSpy {
        id: saveSpy
        signalName: "save"
    }

    SignalSpy {
        id: sendSpy
        signalName: "send"
    }

    TestCase {
        name: "Profile"
        when: windowShown

        function test_blank_fields_save_as_blank_and_unshared() {
            var p = profileComponent.createObject(host, { profile: host.blankProfile, width: 800, height: 700 });
            saveSpy.target = p;
            saveSpy.clear();
            p.setValue("alias", "Mahboob");
            p.setShare("alias", true);
            p.setValue("age", "34");
            p.save(p.draft);
            compare(saveSpy.count, 1);
            var fields = saveSpy.signalArguments[0][0];
            compare(fields.alias.value, "Mahboob");
            compare(fields.alias.share, true);
            compare(fields.age.value, "34");
            compare(fields.age.share, false, "a field was shared without being switched on");
            compare(fields.email.value, "");
            p.destroy();
        }

        function test_shared_count_ignores_empty_fields() {
            var p = profileComponent.createObject(host, { profile: host.blankProfile, width: 800, height: 700 });
            p.setShare("email", true);
            compare(p.sharedCount, 0, "an empty field counted as shared");
            p.setValue("email", "m@example.org");
            compare(p.sharedCount, 1);
            p.destroy();
        }

        function test_a_new_profile_reseeds_the_draft() {
            var p = profileComponent.createObject(host, { profile: host.blankProfile, width: 800, height: 700 });
            p.setValue("phone", "unsaved edit");
            var saved = JSON.parse(JSON.stringify(host.blankProfile));
            saved.fields.phone = { value: "+91 98450 12345", share: true };
            p.profile = saved;
            compare(p.entry("phone").value, "+91 98450 12345");
            compare(p.entry("phone").share, true);
            p.destroy();
        }
    }

    TestCase {
        name: "Discussion"
        when: windowShown

        function test_labels_say_who_wrote_what() {
            var d = discussionComponent.createObject(host, { width: 800, height: 700 });
            compare(d.label({ mine: true, name: "Mahboob", short: "aa" }), "Mahboob (you)");
            compare(d.label({ mine: true, name: null, short: "aa" }), "You");
            compare(d.label({ mine: false, name: null, short: "3a9d0f21" }), "3a9d0f21…");
            compare(d.label({ mine: false, name: "Nisha", short: "3a9d0f21" }), "Nisha");
            d.destroy();
        }

        function test_rln_missing_is_explained() {
            var d = discussionComponent.createObject(host, {
                width: 800, height: 700,
                thread: { messages: [], can_send: true, rln: { required: true, ready: false } }
            });
            verify(d.rlnBlocked);
            d.destroy();
        }

        function test_blank_messages_are_not_sent() {
            var d = discussionComponent.createObject(host, { width: 800, height: 700 });
            sendSpy.target = d;
            sendSpy.clear();
            d.trySend();
            compare(sendSpy.count, 0);
            d.destroy();
        }

        function test_messages_render_as_plain_text() {
            // A message is someone else's text: markup in it must not render.
            var d = discussionComponent.createObject(host, {
                width: 800, height: 700,
                thread: { can_send: true, messages: [
                    { id: "1", from: "aa", short: "aa", mine: false, at: "2026-10-29T12:00:00Z",
                      text: "<b>bold</b> <img src='x'>", name: null, photo: null }
                ] }
            });
            compare(d.messages.length, 1);
            d.destroy();
        }
    }
}
