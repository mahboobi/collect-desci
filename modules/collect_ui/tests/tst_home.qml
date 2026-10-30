// Tests for the dashboard header: refresh and the connection dot.
import QtQuick
import QtTest
import collect
import "../src/qml/screens"

Item {
    id: host
    width: 1100
    height: 700

    Component {
        id: homeComponent
        Home {
            cards: []
        }
    }

    SignalSpy {
        id: refreshSpy
        signalName: "refresh"
    }

    TestCase {
        name: "Home"
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

        function test_each_level_has_its_colour_and_words() {
            var h = homeComponent.createObject(host, { width: 1100, height: 700 });
            var levels = [
                ["online", "Online", Theme.statusOnline],
                ["degraded", "Weak connection", Theme.statusDegraded],
                ["offline", "Offline", Theme.statusOffline]
            ];
            for (var i = 0; i < levels.length; i++) {
                h.connection = { level: levels[i][0], reason: "because" };
                compare(h.connectionLabel, levels[i][1]);
                compare(find(h, "connectionDot").color, levels[i][2]);
                compare(find(h, "connectionLabel").text, levels[i][1]);
            }
            h.destroy();
        }

        function test_a_missing_reading_reads_as_offline() {
            var h = homeComponent.createObject(host, { width: 1100, height: 700, connection: null });
            compare(h.connectionLevel, "offline");
            h.connection = {};
            compare(h.connectionLevel, "offline");
            h.destroy();
        }

        function test_refresh_asks_and_waits_while_busy() {
            var h = homeComponent.createObject(host, { width: 1100, height: 700 });
            refreshSpy.target = h;
            refreshSpy.clear();
            var button = find(h, "refreshButton");
            verify(button.enabled);
            mouseClick(button);
            compare(refreshSpy.count, 1);
            h.refreshing = true;
            verify(!button.enabled, "a second refresh could start while one is running");
            mouseClick(button);
            compare(refreshSpy.count, 1);
            h.destroy();
        }
    }
}
