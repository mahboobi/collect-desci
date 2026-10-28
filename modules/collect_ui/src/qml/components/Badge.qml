// A status pill. Variants come straight from Theme.badge so a status cannot
// pick a colour the design system does not have.
import QtQuick
import collect

Rectangle {
    id: badge

    required property string text
    // One of: done, current, attention, neutral.
    property string variant: "neutral"

    readonly property var tone: Theme.badge[variant] ? Theme.badge[variant] : Theme.badge.neutral

    implicitWidth: label.implicitWidth + Theme.s20
    implicitHeight: Theme.chipHeight
    radius: Theme.rPill
    color: tone.bg

    Text {
        id: label
        anchors.centerIn: parent
        text: badge.text
        font.family: Theme.sans
        font.pixelSize: Theme.meta
        font.weight: Theme.medium
        color: badge.tone.fg
    }
}
