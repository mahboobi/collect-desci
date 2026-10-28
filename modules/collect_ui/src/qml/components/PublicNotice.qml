// The "all data is public" warning. One component, because the warning has to
// read identically on C2 and S1 — a submitter who saw softer wording on the
// form than the creator saw on the experiment was not told the same thing.
import QtQuick
import collect

Rectangle {
    id: notice

    property string detail: "Anyone with the link can read every submission, including yours. Nothing saved on Logos Storage can be deleted."

    width: parent ? parent.width : 0
    implicitHeight: column.implicitHeight + Theme.s28
    radius: Theme.rPanel
    color: Theme.amberTint

    Column {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        anchors.leftMargin: Theme.s16
        anchors.rightMargin: Theme.s16
        spacing: Theme.s6

        Text {
            text: "All data is public"
            font.family: Theme.sans
            font.pixelSize: Theme.body
            font.weight: Theme.semibold
            color: Theme.amber
        }

        Text {
            width: column.width
            wrapMode: Text.Wrap
            text: notice.detail
            font.family: Theme.sans
            font.pixelSize: Theme.small
            color: Theme.ink2
            lineHeight: Theme.bodyLineHeight
            lineHeightMode: Text.ProportionalHeight
        }
    }
}
