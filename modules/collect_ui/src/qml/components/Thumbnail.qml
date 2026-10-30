// An experiment's cover image, 16:9, or graph paper with the title's first
// letter when there is none.
//
// `source` is a data URI from collect_core — the sandbox cannot load
// anything else — or "" for none.
import QtQuick
import collect

Rectangle {
    id: thumb

    property string source: ""
    property string title: ""
    // What the empty state says under the letter, if anything.
    property string emptyNote: ""

    readonly property bool hasImage: thumb.source !== "" && image.status !== Image.Error

    implicitHeight: Math.round(width * 9 / 16)
    radius: Theme.rControl
    color: Theme.accentTintSoft
    border.width: 1
    border.color: Theme.line
    clip: true

    Accessible.role: Accessible.Graphic
    Accessible.name: thumb.hasImage ? "Cover image for " + thumb.title : "No cover image"

    GridPaper {
        anchors.fill: parent
        anchors.margins: 1
        visible: !thumb.hasImage
    }

    Column {
        anchors.centerIn: parent
        visible: !thumb.hasImage
        spacing: Theme.s4

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: thumb.title ? thumb.title.trim().charAt(0).toUpperCase() : "?"
            font.family: Theme.sans
            font.pixelSize: Math.max(24, Math.min(72, thumb.height * 0.36))
            font.weight: Theme.semibold
            color: Theme.accent
            opacity: 0.55
        }

        Text {
            visible: thumb.emptyNote !== ""
            anchors.horizontalCenter: parent.horizontalCenter
            text: thumb.emptyNote
            font.family: Theme.sans
            font.pixelSize: Theme.meta
            color: Theme.ink3
        }
    }

    Image {
        id: image
        anchors.fill: parent
        anchors.margins: 1
        visible: thumb.hasImage
        source: thumb.source
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        smooth: true
        // Never larger than what collect_core made: 960 x 540.
        sourceSize.width: 960
    }
}
