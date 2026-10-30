// "+ Add photos" on a form: the click reaches the picker, each photo that
// comes back is added, and the field never takes more than its maximum.
import QtQuick
import QtTest
import collect
import "../src/qml/fields"

Item {
    id: host
    width: 800
    height: 600

    readonly property var field: ({
        key: "photos", type: "photo", label: "Bird Photos", required: true,
        config: { min: 1, max: 3, max_mb_each: 10 }
    })

    function entry(n) {
        return { cid: "logos:zPhoto" + n, sha256: "ab".repeat(32), mime: "image/jpeg", bytes: 1000 + n };
    }

    Component {
        id: photoComponent
        PhotoField { field: host.field }
    }

    TestCase {
        name: "Photos"
        when: windowShown

        property var asked: null

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
            asked = null;
            var test = this;
            Core.photoAdder = function (maxMb, room, onEach, onPending) {
                test.asked = { maxMb: maxMb, room: room, onEach: onEach, onPending: onPending };
            };
        }

        function cleanup() {
            Core.photoAdder = null;
        }

        function test_the_button_reaches_the_picker_and_photos_arrive() {
            var p = photoComponent.createObject(host, { width: 700 });
            var button = find(p, "addPhotos");
            waitForRendering(p);
            mouseClick(button);
            verify(asked !== null, "clicking + Add photos did nothing");
            compare(asked.maxMb, 10);
            compare(asked.room, 3);

            asked.onPending(2);
            verify(!button.enabled, "a second pick could start while photos are processing");
            var exifSeen = [];
            p.exifRead.connect(function (exif, name) { exifSeen.push(name); });
            asked.onEach({ entry: host.entry(1), exif: { location: { lat: 26.9, lon: 75.8 } }, name: "a.jpg" });
            asked.onEach({ entry: host.entry(2), exif: null, name: "b.jpg" });
            asked.onPending(0);

            compare(p.items.length, 2);
            compare(p.value[0].cid, "logos:zPhoto1");
            compare(exifSeen, ["a.jpg"]);
            verify(button.enabled);
            p.destroy();
        }

        function test_the_room_left_shrinks_and_a_full_field_takes_no_more() {
            var p = photoComponent.createObject(host, { width: 700 });
            p.items = [host.entry(1), host.entry(2)];
            var button = find(p, "addPhotos");
            waitForRendering(p);
            mouseClick(button);
            compare(asked.room, 1);
            asked.onEach({ entry: host.entry(3), exif: null, name: "c.jpg" });
            asked.onEach({ entry: host.entry(4), exif: null, name: "d.jpg" });
            compare(p.items.length, 3, "the field took more than its maximum");
            verify(!button.visible, "the button stayed after the field was full");
            p.destroy();
        }
    }
}
