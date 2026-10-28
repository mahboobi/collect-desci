// A labelled single-line input. Used throughout the field settings.
import QtQuick
import collect

Column {
    id: group

    required property string label
    property string text: ""
    property string hint: ""
    property bool mono: false
    property int maximumLength: 32767
    property var inputHints: Qt.ImhNone

    signal edited(string value)

    spacing: Theme.s6

    Text {
        text: group.label
        font.family: Theme.sans
        font.pixelSize: Theme.meta
        font.weight: Theme.medium
        color: Theme.ink2
    }

    Rectangle {
        width: group.width
        height: Theme.touchTarget
        radius: Theme.rControl
        color: Theme.surface
        border.width: 1
        border.color: input.activeFocus ? Theme.accent : Theme.inputBorder

        TextInput {
            id: input
            anchors.fill: parent
            anchors.margins: Theme.s10
            verticalAlignment: TextInput.AlignVCenter
            text: group.text
            font.family: group.mono ? Theme.mono : Theme.sans
            font.pixelSize: Theme.body
            color: Theme.ink
            selectByMouse: true
            maximumLength: group.maximumLength
            inputMethodHints: group.inputHints
            Accessible.name: group.label
            onTextEdited: group.edited(text)
        }
    }

    Text {
        visible: group.hint !== ""
        width: group.width
        wrapMode: Text.Wrap
        text: group.hint
        font.family: Theme.sans
        font.pixelSize: Theme.meta
        color: Theme.ink3
    }
}
