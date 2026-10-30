// The opt-in wallet on the form: offered only when the profile has one, off
// by default, and sent as a flag — never as the address itself.
import QtQuick
import QtTest
import collect
import "../src/qml/screens"

Item {
    id: host
    width: 1000
    height: 900

    readonly property var protocol: ({
        schema: "collect/protocol@1", id: "birds", version: 1, experiment: "q7k2m3x4w3b5n6tz",
        name: "Birds", instructions: "", allow_multiple: true, allow_drafts: false,
        fields: [ { key: "note", type: "text", label: "Note", required: false, config: {} } ]
    })
    readonly property var experiment: ({ id: "q7k2m3x4w3b5n6tz", title: "Birds of Dhun" })

    Component {
        id: formComponent
        Form {
            experiment: host.experiment
            protocol: host.protocol
        }
    }

    TestCase {
        name: "Wallet"
        when: windowShown

        property var sent: null

        function find(item, name) {
            if (item.objectName === name)
                return item;
            for (var i = 0; i < item.children.length; i++) {
                var hit = find(item.children[i], name);
                if (hit)
                    return hit;
            }
            if (item.contentItem && item.contentItem !== item) {
                var inner = find(item.contentItem, name);
                if (inner)
                    return inner;
            }
            return null;
        }

        function init() {
            sent = null;
            var test = this;
            Core.submitter = function (draft, onData) {
                test.sent = draft;
                onData({ submitted: true, cid: "logos:zX" });
            };
        }

        function cleanup() {
            Core.submitter = null;
        }

        function test_not_offered_without_a_wallet_in_the_profile() {
            var f = formComponent.createObject(host, { width: 900, height: 800 });
            verify(!find(f, "includeWallet").visible);
            f.doSubmit();
            compare(sent.includeWallet, false);
            f.destroy();
        }

        function test_offered_but_off_until_ticked() {
            var f = formComponent.createObject(host, { width: 900, height: 800, wallet: "lez1qexample" });
            verify(find(f, "includeWallet").visible);
            f.doSubmit();
            compare(sent.includeWallet, false, "the wallet was included without being ticked");
            f.destroy();
        }

        function test_ticked_sends_the_flag_and_not_the_address() {
            var f = formComponent.createObject(host, { width: 900, height: 800, wallet: "lez1qexample" });
            find(f, "includeWallet").toggled(true);
            f.doSubmit();
            compare(sent.includeWallet, true);
            verify(sent.wallet === undefined, "the address itself travelled from the UI");
            f.destroy();
        }
    }
}
