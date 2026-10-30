// Cover images: on the card, on the detail page, and the creator's controls.
import QtQuick
import QtTest
import collect
import "../src/qml/screens"
import "../src/qml/components"

Item {
    id: host
    width: 1000
    height: 900

    // A real 1x1 PNG, the shape collect_core hands over.
    readonly property string pixel: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="

    readonly property var experiment: ({
        id: "q7k2m3x4w3b5n6tz", title: "Birds of Dhun", description: "", protocols: [],
        starts_at: "2026-10-20T00:00:00Z", ends_at: "2026-11-20T00:00:00Z", timezone: "Asia/Kolkata",
        creator: { key: "aa", name: "Dr Rao" }
    })

    Component {
        id: thumbComponent
        Thumbnail {}
    }

    Component {
        id: cardComponent
        ExperimentCard {}
    }

    Component {
        id: joinComponent
        Join {}
    }

    TestCase {
        name: "Thumbnail"
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

        function test_it_is_16_by_9_and_shows_the_image_when_there_is_one() {
            var t = thumbComponent.createObject(host, { width: 320, source: host.pixel, title: "Birds" });
            compare(t.height, 180);
            tryCompare(t, "hasImage", true);
            t.destroy();
        }

        function test_without_an_image_it_shows_the_first_letter() {
            var t = thumbComponent.createObject(host, { width: 320, title: "birds of Dhun" });
            verify(!t.hasImage);
            compare(t.Accessible.name, "No cover image");
            t.destroy();
        }

        function test_a_broken_image_falls_back_to_the_grid() {
            var t = thumbComponent.createObject(host, { width: 320, source: "data:image/png;base64,AAAA", title: "x" });
            tryCompare(t, "hasImage", false);
            t.destroy();
        }

        function test_the_card_carries_the_lists_thumbnail() {
            var c = cardComponent.createObject(host, {
                width: 320,
                model: { id: "a", title: "Birds of Dhun", thumbnail: host.pixel, mine: true, protocol_count: 1 }
            });
            compare(find(c, "cardThumbnail").source, host.pixel);
            c.destroy();
        }

        function test_a_submitter_sees_no_empty_cover() {
            var j = joinComponent.createObject(host, { width: 900, height: 800, experiment: host.experiment, mine: false });
            verify(!find(j, "coverImage").parent.visible, "an empty cover was shown to a submitter");
            j.thumbnail = host.pixel;
            verify(find(j, "coverImage").parent.visible);
            verify(!find(j, "chooseCover").visible, "a submitter was offered the creator's controls");
            j.destroy();
        }

        function test_with_a_native_chooser_the_screen_asks_the_backend() {
            var j = joinComponent.createObject(host, {
                width: 900, height: 800, experiment: host.experiment, mine: true, nativePicker: true
            });
            var asked = 0;
            j.requestThumbnailPick.connect(function () { asked++; });
            var choose = find(j, "chooseCover");
            waitForRendering(j);
            mouseClick(choose);
            compare(asked, 1, "the click did not reach the backend's chooser");
            j.destroy();
        }

        function test_a_file_url_from_the_backend_is_shown() {
            // What the backend turns a data: URL into, here pointing at a
            // file that does not exist: the grid shows rather than a hole.
            var t = thumbComponent.createObject(host, { width: 320, source: "file:///nonexistent/x.jpg", title: "x" });
            tryCompare(t, "hasImage", false);
            t.destroy();
        }

        function test_forms_are_listed_without_an_extra_step() {
            var withForm = JSON.parse(JSON.stringify(host.experiment));
            withForm.protocols = [ { id: "migratory-birds", version: 1, cid: "logos:zP", status: "open" } ];
            for (var mine = 0; mine < 2; mine++) {
                var j = joinComponent.createObject(host, {
                    width: 900, height: 800, experiment: withForm, mine: mine === 1
                });
                verify(find(j, "formsSection").visible, "forms hidden behind a step (mine=" + mine + ")");
                j.destroy();
            }
        }

        function test_the_creator_can_add_change_and_remove() {
            var j = joinComponent.createObject(host, { width: 900, height: 800, experiment: host.experiment, mine: true });
            var choose = find(j, "chooseCover");
            verify(choose.visible);
            compare(choose.text, "Add cover image");
            verify(!find(j, "removeCover").visible);

            j.thumbnail = host.pixel;
            compare(choose.text, "Change cover image");
            var removed = 0;
            j.clearThumbnail.connect(function () { removed++; });
            var remove = find(j, "removeCover");
            tryVerify(function () { return remove.visible && remove.width > 0; });
            waitForRendering(j);
            mouseClick(remove);
            compare(removed, 1);

            j.thumbnailBusy = true;
            verify(!choose.enabled, "a second upload could start during the first");
            j.destroy();
        }
    }
}
