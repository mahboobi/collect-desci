// `multi`. Checkbox cards. The value keeps protocol order rather than the
// order someone happened to tick, so two identical answers canonicalise the
// same and produce the same signature.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect

FieldBase {
    id: root

    readonly property var options: config.options ? config.options : []
    readonly property var minimum: config.min
    readonly property var maximum: config.max

    property var chosen: []

    value: chosen.length === 0 ? undefined : chosen
    blank: chosen.length === 0
    problem: {
        if (chosen.length === 0)
            return "";
        if (minimum !== undefined && chosen.length < minimum)
            return "Select at least " + minimum;
        if (maximum !== undefined && chosen.length > maximum)
            return "Select at most " + maximum;
        return "";
    }

    function focusField() { if (list.count > 0) list.itemAt(0).forceActiveFocus(); }

    function toggle(optionValue) {
        var next = [];
        var had = root.chosen.indexOf(optionValue) !== -1;
        for (var i = 0; i < root.options.length; i++) {
            var v = root.options[i].value;
            var on = v === optionValue ? !had : root.chosen.indexOf(v) !== -1;
            if (on)
                next.push(v);
        }
        root.chosen = next;
    }

    Flow {
        width: parent.width
        spacing: Theme.s8

        Repeater {
            id: list
            model: root.options

            delegate: Button {
                id: option
                required property var modelData
                readonly property bool selected: root.chosen.indexOf(modelData.value) !== -1

                height: Theme.touchTarget
                leftPadding: Theme.s14
                rightPadding: Theme.s16
                checkable: true
                checked: selected
                text: modelData.label
                Accessible.name: root.field.label + ": " + modelData.label
                Accessible.checkable: true
                Accessible.checked: selected
                onClicked: root.toggle(modelData.value)

                contentItem: Row {
                    spacing: Theme.s8

                    Rectangle {
                        width: Theme.checkbox
                        height: Theme.checkbox
                        anchors.verticalCenter: parent.verticalCenter
                        radius: Theme.s4
                        color: option.selected ? Theme.accent : Theme.surface
                        border.width: option.selected ? 0 : 1
                        border.color: Theme.inputBorder

                        Text {
                            anchors.centerIn: parent
                            visible: option.selected
                            text: "\u2713"
                            font.pixelSize: Theme.meta
                            color: Theme.textOnAccent
                        }
                    }

                    Text {
                        text: option.text
                        anchors.verticalCenter: parent.verticalCenter
                        font.family: Theme.sans
                        font.pixelSize: Theme.body
                        color: Theme.ink
                    }
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
