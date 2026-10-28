// `datetime`. Stored as RFC 3339 **with an offset** — collect_core rejects a
// timestamp without one, because without it the value names no instant.
import QtQuick
import QtQuick.Controls
import collect

FieldBase {
    id: root

    readonly property bool noFuture: config.no_future === true
    readonly property var fillFrom: config.fill_from ? config.fill_from : []
    readonly property bool canFillFromPhoto: fillFrom.indexOf("photo") !== -1
    readonly property bool canFillFromNow: fillFrom.indexOf("now") !== -1

    // Where the current value came from, which the artboard shows as
    // "From IMG_3108.jpg".
    property string source: ""

    value: stamp === "" ? undefined : stamp
    property string stamp: ""

    problem: {
        if (stamp === "")
            return "";
        var t = Date.parse(stamp);
        if (isNaN(t))
            return "Not a valid date and time";
        if (noFuture && t > Date.now())
            return "Cannot be in the future";
        return "";
    }

    function focusField() { input.forceActiveFocus(); }

    // Local time with the machine's own offset, which is the only offset the
    // UI can honestly claim.
    function nowWithOffset() {
        var d = new Date();
        function two(n) { return (n < 10 ? "0" : "") + n; }
        var tz = -d.getTimezoneOffset();
        var sign = tz >= 0 ? "+" : "-";
        var abs = Math.abs(tz);
        return d.getFullYear() + "-" + two(d.getMonth() + 1) + "-" + two(d.getDate())
             + "T" + two(d.getHours()) + ":" + two(d.getMinutes()) + ":" + two(d.getSeconds())
             + sign + two(Math.floor(abs / 60)) + ":" + two(abs % 60);
    }

    // Called by the renderer when a photo yields a datetime. Only accepted
    // when the protocol allows it.
    function fillFromPhoto(photoStamp, photoName) {
        if (!canFillFromPhoto || !photoStamp)
            return;
        root.stamp = photoStamp;
        root.source = photoName ? "From " + photoName : "From the photo";
    }

    Column {
        width: parent.width
        spacing: Theme.s8

        Rectangle {
            width: parent.width
            height: Theme.touchTarget
            radius: Theme.rControl
            color: Theme.surface
            border.width: 1
            border.color: input.activeFocus ? Theme.accent
                        : (root.problem !== "" ? Theme.amber : Theme.inputBorder)

            TextInput {
                id: input
                anchors.fill: parent
                anchors.margins: Theme.s10
                verticalAlignment: TextInput.AlignVCenter
                text: root.stamp
                font.family: Theme.mono
                font.pixelSize: Theme.body
                color: Theme.ink
                selectByMouse: true
                Accessible.name: root.field.label
                Accessible.description: "Date and time with a UTC offset, for example 2026-10-28T07:42:00+05:30"
                onTextEdited: {
                    root.stamp = text;
                    root.source = "";
                }
            }
        }

        Row {
            spacing: Theme.s12

            Button {
                id: nowButton
                visible: root.canFillFromNow
                height: Theme.chipHeight
                text: "Use now"
                Accessible.name: "Set " + root.field.label + " to now"
                onClicked: {
                    root.stamp = root.nowWithOffset();
                    root.source = "";
                }
                contentItem: Text {
                    text: nowButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    color: Theme.accent
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rPill
                    color: Theme.accentTint
                }
            }

            Text {
                visible: root.source !== ""
                text: root.source
                anchors.verticalCenter: parent.verticalCenter
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
            }
        }
    }
}
