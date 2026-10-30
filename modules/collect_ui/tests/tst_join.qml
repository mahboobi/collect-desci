// Joining by link and following one's own submissions, at the screen
// boundary. Fetching, checking and the outbox rules are tested in Rust.
import QtQuick
import QtTest
import collect
import "../src/qml/screens"

Item {
    id: host
    width: 1100
    height: 900

    readonly property var experiment: ({
        id: "q7k2m3x4w3b5n6tz", title: "Birds of Dhun", description: "", protocols: [],
        starts_at: "2026-10-20T00:00:00Z", ends_at: "2026-11-20T00:00:00Z", creator: { key: "aa", name: "" }
    })

    Component { id: homeComponent; Home { cards: [] } }
    Component { id: joinComponent; Join {} }

    TestCase {
        name: "JoinByLink"
        when: windowShown

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

        function test_join_sends_the_typed_link_and_shows_why_it_failed() {
            var h = homeComponent.createObject(host, { width: 1000, height: 800 });
            var join = find(h, "joinButton");
            verify(!join.enabled, "join offered with no link");
            find(h, "joinLinkInput").edited("  logos-collect://exp/q7k2m3x4w3b5n6tz?m=logos:zM  ");
            verify(join.enabled);
            var asked = [];
            h.joinLink.connect(function (link) { asked.push(link); });
            waitForRendering(h);
            mouseClick(join);
            compare(asked, ["logos-collect://exp/q7k2m3x4w3b5n6tz?m=logos:zM"], "the link was not trimmed");

            h.joinBusy = true;
            verify(!join.enabled, "a second fetch could start during the first");
            h.joinBusy = false;
            h.joinError = "that is not a Collect link";
            compare(find(h, "joinLinkInput").hint, "that is not a Collect link");
            h.destroy();
        }

        function test_a_submitter_sees_waiting_then_received() {
            var j = joinComponent.createObject(host, { width: 900, height: 800, experiment: host.experiment, mine: false,
                mySubmissions: [ { cid: "logos:z1", protocol: "birds", submitted_at: "2026-10-28T09:00:00Z",
                                   state: "waiting", attempts: 2, last_error: "no RLN membership" } ] });
            var list = find(j, "mySubmissions");
            verify(list.visible);
            j.mySubmissions = [ { cid: "logos:z1", protocol: "birds", submitted_at: "2026-10-28T09:00:00Z",
                                  state: "received", attempts: 2 } ];
            verify(list.visible);
            j.destroy();
        }

        function test_the_creator_sees_no_outbox() {
            var j = joinComponent.createObject(host, { width: 900, height: 800, experiment: host.experiment, mine: true,
                mySubmissions: [ { cid: "logos:z1", protocol: "birds", submitted_at: "2026-10-28T09:00:00Z",
                                   state: "waiting", attempts: 1 } ] });
            verify(!find(j, "mySubmissions").visible);
            j.destroy();
        }
    }
}
