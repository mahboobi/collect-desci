// A labelled checkbox with optional explanatory text.
//
// `toggled` fires only when a person clicks it, which is what a caller that
// *binds* `on` must listen to. Watching `onChanged` instead is a binding
// loop: the write-back changes the bound expression, which re-evaluates `on`,
// which fires `onChanged` again. Builder's "Required" row did exactly that —
// `QML FieldSettings: Binding loop detected for property "field"`.
import QtQuick
import QtQuick.Controls
import collect

Item {
    id: row

    required property string label
    property string hint: ""
    property bool on: false

    /// Emitted only for a click, never for a binding re-evaluation.
    signal toggled(bool on)

    implicitHeight: layout.implicitHeight

    Row {
        id: layout
        width: parent.width
        spacing: Theme.s12

        Button {
            id: box
            width: Theme.touchTarget
            height: Theme.touchTarget
            checkable: true
            checked: row.on
            Accessible.name: row.label
            Accessible.checkable: true
            Accessible.checked: row.on
            onClicked: {
                row.on = !row.on;
                row.toggled(row.on);
            }

            contentItem: Rectangle {
                anchors.centerIn: parent
                width: Theme.checkbox
                height: Theme.checkbox
                radius: Theme.s4
                color: row.on ? Theme.accent : Theme.surface
                border.width: row.on ? 0 : 1
                border.color: Theme.inputBorder

                Text {
                    anchors.centerIn: parent
                    visible: row.on
                    text: "\u2713"
                    font.pixelSize: Theme.meta
                    color: Theme.textOnAccent
                }
            }
            background: null
        }

        Column {
            anchors.verticalCenter: parent.verticalCenter
            width: layout.width - Theme.touchTarget - Theme.s12
            spacing: Theme.s2

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: row.label
                font.family: Theme.sans
                font.pixelSize: Theme.body
                color: Theme.ink
            }

            Text {
                visible: row.hint !== ""
                width: parent.width
                wrapMode: Text.Wrap
                text: row.hint
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
            }
        }
    }
}
