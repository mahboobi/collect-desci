// `number`. Stepper plus typed entry, with the unit shown beside it.
import QtQuick
import QtQuick.Controls
import collect

FieldBase {
    id: root

    readonly property var minimum: config.min
    readonly property var maximum: config.max
    readonly property bool integerOnly: config.integer === true
    readonly property string unit: config.unit ? config.unit : ""

    readonly property real step: integerOnly ? 1 : 0.1

    value: input.text === "" ? undefined : Number(input.text)
    problem: {
        if (input.text === "")
            return "";
        var n = Number(input.text);
        if (isNaN(n))
            return "Enter a number";
        if (integerOnly && n % 1 !== 0)
            return "Whole numbers only";
        if (minimum !== undefined && n < minimum)
            return "At least " + minimum;
        if (maximum !== undefined && n > maximum)
            return "At most " + maximum;
        return "";
    }

    function focusField() { input.forceActiveFocus(); }

    function nudge(by) {
        var n = input.text === "" ? (root.minimum !== undefined ? root.minimum : 0) : Number(input.text);
        if (isNaN(n))
            n = 0;
        n = n + by;
        if (root.minimum !== undefined && n < root.minimum)
            n = root.minimum;
        if (root.maximum !== undefined && n > root.maximum)
            n = root.maximum;
        // Keep the text free of float noise, e.g. 0.30000000000000004.
        input.text = root.integerOnly ? String(Math.round(n)) : String(Math.round(n * 100) / 100);
    }

    Row {
        spacing: Theme.s8

        Button {
            id: minusButton
            width: Theme.touchTarget
            height: Theme.touchTarget
            text: "\u2212"
            Accessible.name: "Decrease " + root.field.label
            onClicked: root.nudge(-root.step)
            background: Rectangle {
                radius: Theme.rControl
                color: minusButton.down ? Theme.surfaceSunken : Theme.surface
                border.width: 1
                border.color: Theme.inputBorder
            }
        }

        Rectangle {
            width: 96
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
                horizontalAlignment: TextInput.AlignHCenter
                font.family: Theme.mono
                font.pixelSize: Theme.body
                color: Theme.ink
                selectByMouse: true
                // Lets a decimal be typed even when the field is integer-only,
                // so the complaint explains the rule instead of the keystroke
                // silently vanishing.
                inputMethodHints: Qt.ImhFormattedNumbersOnly
                Accessible.name: root.field.label
            }
        }

        Button {
            id: plusButton
            width: Theme.touchTarget
            height: Theme.touchTarget
            text: "+"
            Accessible.name: "Increase " + root.field.label
            onClicked: root.nudge(root.step)
            background: Rectangle {
                radius: Theme.rControl
                color: plusButton.down ? Theme.surfaceSunken : Theme.surface
                border.width: 1
                border.color: Theme.inputBorder
            }
        }

        Text {
            visible: root.unit !== ""
            text: root.unit
            anchors.verticalCenter: parent.verticalCenter
            font.family: Theme.sans
            font.pixelSize: Theme.body
            color: Theme.ink3
        }
    }
}
