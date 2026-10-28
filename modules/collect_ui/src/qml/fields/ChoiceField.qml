// `choice`. Radio cards, one selection, value is the option `value` and never
// its label.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect

FieldBase {
    id: root

    readonly property var options: config.options ? config.options : []

    value: picked
    property var picked: undefined

    function focusField() { if (list.count > 0) list.itemAt(0).forceActiveFocus(); }

    Flow {
        width: parent.width
        spacing: Theme.s8

        Repeater {
            id: list
            model: root.options

            delegate: Button {
                id: option
                required property var modelData
                readonly property bool selected: root.picked === modelData.value

                height: Theme.touchTarget
                leftPadding: Theme.s16
                rightPadding: Theme.s16
                checkable: true
                checked: selected
                text: modelData.label
                // The stored value, not the label, is what a reader of the
                // submission sees — worth having in the accessible name.
                Accessible.name: root.field.label + ": " + modelData.label
                onClicked: root.picked = modelData.value

                contentItem: Text {
                    text: option.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: option.selected ? Theme.medium : Theme.regular
                    color: option.selected ? Theme.accent : Theme.ink
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: option.selected ? Theme.accentTintSoft : Theme.surface
                    border.width: option.selected ? 2 : 1
                    border.color: option.selected ? Theme.accent : Theme.inputBorder
                }
            }
        }
    }
}
