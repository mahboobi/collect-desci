// `photo`. The UI never reads or uploads a file itself — the QML sandbox has
// no filesystem and no network. Picking a file hands the path to collect_core,
// which strips the metadata and returns the media entry plus whatever EXIF it
// read. What comes back here is already cleaned.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect

FieldBase {
    id: root

    readonly property int minimum: config.min !== undefined ? config.min : 0
    // 10 is the contract ceiling; a protocol cannot raise it.
    readonly property int maximum: Math.min(config.max !== undefined ? config.max : 10, 10)
    readonly property real maxMbEach: config.max_mb_each !== undefined ? config.max_mb_each : 25
    readonly property var fillFromExif: config.fill_from_exif ? config.fill_from_exif : []

    // Media entries as they will appear in the submission.
    property var items: []

    value: items.length === 0 ? undefined : items
    blank: items.length === 0
    problem: {
        if (items.length === 0)
            return "";
        if (items.length > maximum)
            return "At most " + maximum + " photos";
        if (items.length < minimum)
            return "At least " + minimum + " photos";
        return "";
    }

    // The renderer listens for these to prefill `location` and `datetime`.
    signal exifRead(var exif, string name)

    function focusField() { addButton.forceActiveFocus(); }

    function addProcessed(entry, exif, name) {
        if (root.items.length >= root.maximum)
            return;
        var next = root.items.slice();
        next.push(entry);
        root.items = next;
        if (exif)
            root.exifRead(exif, name);
    }

    function removeAt(i) {
        var next = root.items.slice();
        next.splice(i, 1);
        root.items = next;
    }

    Column {
        width: parent.width
        spacing: Theme.s10

        Repeater {
            model: root.items

            delegate: Rectangle {
                id: entry
                required property var modelData
                required property int index

                width: root.width
                height: 56
                radius: Theme.rInner
                color: Theme.surfaceSunken

                Row {
                    anchors.fill: parent
                    anchors.margins: Theme.s12
                    spacing: Theme.s12

                    Column {
                        anchors.verticalCenter: parent.verticalCenter
                        width: entry.width - removeButton.width - Theme.s24 - Theme.s24
                        spacing: Theme.s2

                        Text {
                            text: Core.shortCid(entry.modelData.cid)
                            font.family: Theme.mono
                            font.pixelSize: Theme.small
                            color: Theme.ink
                            elide: Text.ElideMiddle
                            width: parent.width
                        }
                        Text {
                            // Dimensions and size, the two things a submitter
                            // can sanity-check about a cleaned file.
                            text: (entry.modelData.width && entry.modelData.height
                                   ? entry.modelData.width + " x " + entry.modelData.height + " \u00b7 " : "")
                                  + Math.round(entry.modelData.bytes / 1024) + " kB \u00b7 metadata removed"
                            font.family: Theme.sans
                            font.pixelSize: Theme.meta
                            color: Theme.ink3
                        }
                    }

                    Button {
                        id: removeButton
                        width: Theme.touchTarget
                        height: Theme.touchTarget
                        anchors.verticalCenter: parent.verticalCenter
                        text: "Remove"
                        Accessible.name: "Remove photo " + (entry.index + 1)
                        onClicked: root.removeAt(entry.index)
                        contentItem: Text {
                            text: "\u2715"
                            font.pixelSize: Theme.body
                            color: Theme.ink2
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                        background: Rectangle {
                            radius: Theme.rControl
                            color: removeButton.down ? Theme.line : "transparent"
                        }
                    }
                }
            }
        }

        Button {
            id: addButton
            visible: root.items.length < root.maximum
            height: Theme.touchTarget
            width: parent.width
            text: "+ Add photos"
            Accessible.name: "Add photos to " + root.field.label
            // Wiring this to a file dialog belongs with the live
            // collect_core.submissions.addMedia job, not fixtures.
            onClicked: root.requestAdd()

            contentItem: Text {
                text: addButton.text
                font.family: Theme.sans
                font.pixelSize: Theme.body
                font.weight: Theme.medium
                color: Theme.accent
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
            }
            background: Rectangle {
                radius: Theme.rControl
                color: "transparent"
                border.width: 1
                border.color: Theme.dashed
            }
        }

        // The promise the pipeline actually keeps, stated where someone is
        // about to hand over a photo.
        Text {
            width: parent.width
            wrapMode: Text.Wrap
            text: "Hidden metadata such as camera model and GPS is removed from photos before they are saved."
            font.family: Theme.sans
            font.pixelSize: Theme.meta
            color: Theme.ink3
        }
    }

    signal requestAdd()
}
