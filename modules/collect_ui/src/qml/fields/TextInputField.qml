// `text` and `longtext`. One delegate: they differ only in height and in the
// max_length default, which collect_core also treats as one Config::Text.
import QtQuick
import QtQuick.Controls
import collect

FieldBase {
    id: root

    readonly property bool multiline: field.type === "longtext"
    readonly property int maxLength: config.max_length !== undefined
                                     ? config.max_length
                                     : (multiline ? 5000 : 200)

    value: input.text === "" ? undefined : input.text
    problem: input.text.length > maxLength
             ? "At most " + maxLength + " characters (" + input.text.length + " now)"
             : ""

    function focusField() { input.forceActiveFocus(); }

    Column {
        width: parent.width
        spacing: Theme.s6

        Rectangle {
            width: parent.width
            height: root.multiline ? 112 : Theme.touchTarget
            radius: Theme.rControl
            color: Theme.surface
            border.width: 1
            border.color: input.activeFocus ? Theme.accent
                        : (root.problem !== "" ? Theme.amber : Theme.inputBorder)

            // One scrollable editor for longtext, a plain one otherwise.
            ScrollView {
                anchors.fill: parent
                anchors.margins: Theme.s10
                clip: true

                TextArea {
                    id: input
                    // The character count is advisory while typing; the hard
                    // limit is the contract's, enforced in collect_core.
                    wrapMode: root.multiline ? TextEdit.Wrap : TextEdit.NoWrap
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    color: Theme.ink
                    background: null
                    padding: 0
                    selectByMouse: true
                    Accessible.name: root.field.label
                    Accessible.description: root.field.help ? root.field.help : ""
                }
            }
        }

        Text {
            visible: root.multiline || input.text.length > root.maxLength * 0.8
            text: input.text.length + " / " + root.maxLength
            font.family: Theme.mono
            font.pixelSize: Theme.meta
            color: input.text.length > root.maxLength ? Theme.amber : Theme.ink3
        }
    }
}
