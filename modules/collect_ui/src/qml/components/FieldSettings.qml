// Settings for the field selected in the builder.
//
// Only the members each type actually has, so the panel cannot produce a
// config the contract rejects. `key` is editable while the form is unpublished
// but called out as permanent, because once a version is published the keys
// are what every submission is keyed by.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect
import "../collect/FieldKinds.js" as Kinds

Column {
    id: panel

    /// The selected field, or null.
    property var field: null

    signal setMember(string member, var value)
    signal setConfig(string member, var value)

    readonly property string kind: panel.field ? panel.field.type : ""
    readonly property var config: panel.field && panel.field.config ? panel.field.config : ({})

    spacing: Theme.s16

    Text {
        text: panel.field ? Kinds.labelFor(panel.kind) + " settings" : ""
        font.family: Theme.sans
        font.pixelSize: Theme.sectionTitle
        font.weight: Theme.semibold
        color: Theme.ink
    }

    // ---- every type ----
    LabelledInput {
        width: panel.width
        label: "Question shown to submitters"
        text: panel.field ? panel.field.label : ""
        maximumLength: 120
        onEdited: function (v) { panel.setMember("label", v); }
    }

    LabelledInput {
        width: panel.width
        label: "Help text \u00b7 optional"
        text: panel.field && panel.field.help ? panel.field.help : ""
        maximumLength: 300
        onEdited: function (v) { panel.setMember("help", v); }
    }

    LabelledInput {
        width: panel.width
        label: "Stored key"
        hint: "Every submission is keyed by this. It cannot change once the form is published."
        text: panel.field ? panel.field.key : ""
        mono: true
        maximumLength: 40
        onEdited: function (v) { panel.setMember("key", v); }
    }

    CheckRow {
        width: panel.width
        label: "Required"
        on: panel.field ? panel.field.required === true : false
        // `toggled`, not `onChanged`: `on` is bound to the field, so writing
        // back on every change of it loops.
        onToggled: function (value) { panel.setMember("required", value); }
    }

    // ---- text and longtext ----
    LabelledInput {
        visible: panel.kind === "text" || panel.kind === "longtext"
        width: panel.width
        label: "Maximum characters"
        text: panel.config.max_length !== undefined ? String(panel.config.max_length) : ""
        mono: true
        inputHints: Qt.ImhDigitsOnly
        onEdited: function (v) {
            var n = parseInt(v, 10);
            if (!isNaN(n) && n > 0)
                panel.setConfig("max_length", n);
        }
    }

    // ---- number ----
    Row {
        visible: panel.kind === "number"
        spacing: Theme.s12

        LabelledInput {
            width: 150
            label: "Minimum \u00b7 optional"
            text: panel.config.min !== undefined ? String(panel.config.min) : ""
            mono: true
            onEdited: function (v) {
                var n = parseFloat(v);
                if (!isNaN(n))
                    panel.setConfig("min", n);
            }
        }

        LabelledInput {
            width: 150
            label: "Maximum \u00b7 optional"
            text: panel.config.max !== undefined ? String(panel.config.max) : ""
            mono: true
            onEdited: function (v) {
                var n = parseFloat(v);
                if (!isNaN(n))
                    panel.setConfig("max", n);
            }
        }

        LabelledInput {
            width: 150
            label: "Unit \u00b7 optional"
            text: panel.config.unit !== undefined ? panel.config.unit : ""
            maximumLength: 30
            onEdited: function (v) { panel.setConfig("unit", v); }
        }
    }

    CheckRow {
        visible: panel.kind === "number"
        width: panel.width
        label: "Whole numbers only"
        on: panel.config.integer === true
        onToggled: function (value) { panel.setConfig("integer", value); }
    }

    // ---- choice and multi ----
    Column {
        visible: panel.kind === "choice" || panel.kind === "multi"
        width: panel.width
        spacing: Theme.s8

        Text {
            text: "Options"
            font.family: Theme.sans
            font.pixelSize: Theme.meta
            font.weight: Theme.medium
            color: Theme.ink2
        }

        Text {
            width: parent.width
            wrapMode: Text.Wrap
            text: "One per line. At least two are needed. Removing an option after publishing orphans the submissions that chose it."
            font.family: Theme.sans
            font.pixelSize: Theme.meta
            color: Theme.ink3
        }

        Rectangle {
            width: parent.width
            height: 120
            radius: Theme.rControl
            color: Theme.surface
            border.width: 1
            border.color: optionsArea.activeFocus ? Theme.accent : Theme.inputBorder

            ScrollView {
                anchors.fill: parent
                anchors.margins: Theme.s10
                clip: true

                TextArea {
                    id: optionsArea
                    wrapMode: TextEdit.Wrap
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    color: Theme.ink
                    background: null
                    padding: 0
                    selectByMouse: true
                    Accessible.name: "Options, one per line"
                    // Labels in, values derived — a creator should not have to
                    // think about the stored value, and deriving it keeps it
                    // inside the contract's pattern.
                    text: {
                        var list = panel.config.options ? panel.config.options : [];
                        return list.map(function (o) { return o.label; }).join("\n");
                    }
                    onTextEdited: {
                        var lines = text.split("\n").filter(function (l) { return l.trim() !== ""; });
                        var taken = [];
                        var options = lines.map(function (l) {
                            var value = Kinds.keyFrom(l, taken);
                            taken.push(value);
                            return { value: value, label: l.trim() };
                        });
                        panel.setConfig("options", options);
                    }
                }
            }
        }
    }

    // ---- datetime ----
    CheckRow {
        visible: panel.kind === "datetime"
        width: panel.width
        label: "Cannot be in the future"
        on: panel.config.no_future === true
        onToggled: function (value) { panel.setConfig("no_future", value); }
    }

    // ---- photo ----
    Row {
        visible: panel.kind === "photo"
        spacing: Theme.s12

        LabelledInput {
            width: 140
            label: "Fewest"
            text: panel.config.min !== undefined ? String(panel.config.min) : ""
            mono: true
            inputHints: Qt.ImhDigitsOnly
            onEdited: function (v) {
                var n = parseInt(v, 10);
                if (!isNaN(n) && n >= 0 && n <= 10)
                    panel.setConfig("min", n);
            }
        }

        LabelledInput {
            width: 140
            label: "Most \u00b7 up to 10"
            text: panel.config.max !== undefined ? String(panel.config.max) : ""
            mono: true
            inputHints: Qt.ImhDigitsOnly
            onEdited: function (v) {
                var n = parseInt(v, 10);
                if (!isNaN(n) && n >= 1 && n <= 10)
                    panel.setConfig("max", n);
            }
        }

        LabelledInput {
            width: 160
            label: "MB each"
            text: panel.config.max_mb_each !== undefined ? String(panel.config.max_mb_each) : ""
            mono: true
            onEdited: function (v) {
                var n = parseFloat(v);
                if (!isNaN(n) && n >= 1 && n <= 100)
                    panel.setConfig("max_mb_each", n);
            }
        }
    }

    // ---- location ----
    Column {
        visible: panel.kind === "location"
        width: panel.width
        spacing: Theme.s10

        CheckRow {
            width: parent.width
            label: "Round to about 1 km"
            hint: "Strongly recommended. Everything is public and nothing on Logos Storage can be deleted, so an exact position saved once is public forever."
            on: panel.config.store === "rounded_1km"
            onToggled: function (value) { panel.setConfig("store", value ? "rounded_1km" : "exact"); }
        }

        Text {
            width: parent.width
            wrapMode: Text.Wrap
            visible: panel.config.store === "exact"
            text: "This form will store exact positions."
            font.family: Theme.sans
            font.pixelSize: Theme.small
            color: Theme.amber
        }
    }
}
