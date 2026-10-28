// `yesno`. Two radio cards rather than a switch: a switch has no "unanswered"
// state, and a required yesno has to be able to read as unanswered.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect

FieldBase {
    id: root

    // false is an answer, so only `undefined` counts as blank.
    blank: value === undefined
    value: group.picked

    function focusField() { if (options.count > 0) options.itemAt(0).forceActiveFocus(); }

    QtObject {
        id: group
        property var picked: undefined
    }

    Row {
        spacing: Theme.s8

        Repeater {
            id: options
            model: [{ label: "Yes", v: true }, { label: "No", v: false }]

            delegate: Button {
                id: option
                required property var modelData
                readonly property bool selected: group.picked === modelData.v

                height: Theme.touchTarget
                leftPadding: Theme.s20
                rightPadding: Theme.s20
                text: modelData.label
                checkable: true
                checked: selected
                Accessible.name: root.field.label + ": " + modelData.label
                onClicked: group.picked = modelData.v

                contentItem: Text {
                    text: option.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: option.selected ? Theme.medium : Theme.regular
                    color: option.selected ? Theme.accent : Theme.ink
                    horizontalAlignment: Text.AlignHCenter
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
