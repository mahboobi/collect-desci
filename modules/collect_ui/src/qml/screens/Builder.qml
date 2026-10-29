// C3 — the protocol builder.
//
// Compose a form: name, instructions, the fields, and the submission options.
// The preview at the bottom is the real FormRenderer reading the real
// protocol JSON, so what a creator sees here is what a submitter gets — there
// is no second rendering path to drift.
//
// collect_core validates on save and refuses a form the renderer could not
// draw, so this screen aims to make an invalid form hard to build rather than
// to re-implement that check.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect
import "../components"
import "../collect/FieldKinds.js" as Kinds

Flickable {
    id: screen

    required property string experimentId
    /// An existing protocol to edit, or null for a new one.
    property var existing: null

    /// Per-field problems from collect_core, keyed by field name.
    property var problems: ({})
    property bool saving: false

    signal save(string json)
    signal cancelled()

    // The form being built. A plain array of field objects, which is exactly
    // what a protocol's `fields` member is.
    property var fields: []
    property int selected: -1

    readonly property bool complete: nameField.text.trim() !== "" && screen.fields.length > 0

    contentWidth: width
    contentHeight: body.implicitHeight + Theme.s48 * 2
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    Component.onCompleted: {
        if (screen.existing) {
            nameField.text = screen.existing.name ? screen.existing.name : "";
            instructionsField.text = screen.existing.instructions ? screen.existing.instructions : "";
            screen.fields = screen.existing.fields ? JSON.parse(JSON.stringify(screen.existing.fields)) : [];
            multipleBox.on = screen.existing.allow_multiple !== false;
            draftsBox.on = screen.existing.allow_drafts !== false;
            screen.selected = screen.fields.length > 0 ? 0 : -1;
        }
    }

    function takenKeys() {
        return screen.fields.map(function (f) { return f.key; });
    }

    function addField(type) {
        var next = screen.fields.slice();
        next.push(Kinds.newField(type, screen.takenKeys()));
        screen.fields = next;
        screen.selected = next.length - 1;
    }

    function removeAt(i) {
        var next = screen.fields.slice();
        next.splice(i, 1);
        screen.fields = next;
        screen.selected = Math.min(i, next.length - 1);
    }

    function moveBy(i, delta) {
        var target = i + delta;
        if (target < 0 || target >= screen.fields.length)
            return;
        var next = screen.fields.slice();
        var held = next[i];
        next[i] = next[target];
        next[target] = held;
        screen.fields = next;
        screen.selected = target;
    }

    /// Replace one member of the selected field. Reassigns the whole array so
    /// the preview and the list both re-read it.
    function setOnSelected(member, value) {
        if (screen.selected < 0)
            return;
        var next = screen.fields.slice();
        var field = JSON.parse(JSON.stringify(next[screen.selected]));
        field[member] = value;
        next[screen.selected] = field;
        screen.fields = next;
    }

    function setConfigOnSelected(member, value) {
        if (screen.selected < 0)
            return;
        var next = screen.fields.slice();
        var field = JSON.parse(JSON.stringify(next[screen.selected]));
        if (!field.config)
            field.config = {};
        field.config[member] = value;
        next[screen.selected] = field;
        screen.fields = next;
    }

    /// The protocol as collect_core will receive it.
    function draft() {
        return {
            experimentId: screen.experimentId,
            protocol: {
                id: screen.existing && screen.existing.id ? screen.existing.id : undefined,
                name: nameField.text.trim(),
                instructions: instructionsField.text,
                fields: screen.fields,
                allow_multiple: multipleBox.on,
                allow_drafts: draftsBox.on
            }
        };
    }

    /// What the preview renders: a protocol-shaped object, so FormRenderer
    /// needs no special case.
    readonly property var previewProtocol: ({
        schema: "collect/protocol@1",
        id: "preview",
        version: 1,
        experiment: screen.experimentId,
        name: nameField.text,
        instructions: instructionsField.text,
        fields: screen.fields,
        allow_multiple: true,
        allow_drafts: true
    })

    Column {
        id: body
        x: Theme.s32
        y: Theme.s32
        width: Math.min(screen.width - Theme.s32 * 2, 860)
        spacing: Theme.s24

        // The same back control the other screens have. Cancel sits at the
        // bottom of a long form, which is no use to someone who has scrolled
        // and changed their mind.
        Button {
            id: backButton
            height: Theme.chipHeight
            leftPadding: Theme.s12
            rightPadding: Theme.s14
            text: "\u2190 Back to the experiment"
            Accessible.name: "Back to the experiment"
            onClicked: screen.cancelled()
            contentItem: Text {
                text: backButton.text
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: Theme.ink2
                verticalAlignment: Text.AlignVCenter
            }
            background: Rectangle {
                radius: Theme.rPill
                color: backButton.down ? Theme.surfaceSunken : "transparent"
            }
        }

        Text {
            text: screen.existing ? "Edit form" : "New form"
            font.family: Theme.sans
            font.pixelSize: Theme.pageTitle
            font.weight: Theme.semibold
            font.letterSpacing: Theme.titleSpacing
            color: Theme.ink
        }

        // ---- name ----
        Column {
            width: parent.width
            spacing: Theme.s8

            Text {
                text: "Form name \u00b7 required"
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                font.weight: Theme.medium
                color: Theme.ink2
            }

            Rectangle {
                width: parent.width
                height: Theme.touchTarget
                radius: Theme.rControl
                color: Theme.surface
                border.width: 1
                border.color: nameField.activeFocus ? Theme.accent
                            : (screen.problems.name ? Theme.amber : Theme.inputBorder)

                TextInput {
                    id: nameField
                    anchors.fill: parent
                    anchors.margins: Theme.s10
                    verticalAlignment: TextInput.AlignVCenter
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    color: Theme.ink
                    selectByMouse: true
                    maximumLength: 120
                    Accessible.name: "Form name"
                }
            }

            Text {
                visible: !!screen.problems.name
                text: screen.problems.name ? screen.problems.name : ""
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: Theme.amber
            }
        }

        // ---- instructions ----
        Column {
            width: parent.width
            spacing: Theme.s8

            Text {
                text: "Instructions for submitters \u00b7 optional"
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                font.weight: Theme.medium
                color: Theme.ink2
            }

            Rectangle {
                width: parent.width
                height: 96
                radius: Theme.rControl
                color: Theme.surface
                border.width: 1
                border.color: instructionsField.activeFocus ? Theme.accent : Theme.inputBorder

                ScrollView {
                    anchors.fill: parent
                    anchors.margins: Theme.s10
                    clip: true

                    TextArea {
                        id: instructionsField
                        wrapMode: TextEdit.Wrap
                        font.family: Theme.sans
                        font.pixelSize: Theme.body
                        color: Theme.ink
                        background: null
                        padding: 0
                        selectByMouse: true
                        Accessible.name: "Instructions for submitters"
                    }
                }
            }
        }

        // ---- the fields ----
        Column {
            width: parent.width
            spacing: Theme.s12

            Text {
                text: "Fields"
                font.family: Theme.sans
                font.pixelSize: Theme.sectionTitle
                font.weight: Theme.semibold
                color: Theme.ink
            }

            Text {
                visible: screen.fields.length === 0
                text: "No fields yet. Add one below."
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: Theme.ink3
            }

            Repeater {
                model: screen.fields

                delegate: Rectangle {
                    id: row
                    required property var modelData
                    required property int index
                    readonly property bool active: screen.selected === row.index

                    width: body.width
                    height: Theme.touchTarget + Theme.s12
                    radius: Theme.rInner
                    color: row.active ? Theme.accentTintSoft : Theme.surface
                    border.width: row.active ? 2 : 1
                    border.color: row.active ? Theme.accent : Theme.line

                    TapHandler { onTapped: screen.selected = row.index }

                    Row {
                        anchors.fill: parent
                        anchors.margins: Theme.s10
                        spacing: Theme.s8

                        Column {
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - Theme.touchTarget * 3 - Theme.s24
                            spacing: Theme.s2

                            Text {
                                width: parent.width
                                elide: Text.ElideRight
                                text: (row.modelData.label ? row.modelData.label : "(no label)")
                                      + (row.modelData.required ? " \u00b7 required" : "")
                                font.family: Theme.sans
                                font.pixelSize: Theme.body
                                font.weight: Theme.medium
                                color: Theme.ink
                            }

                            Text {
                                text: Kinds.labelFor(row.modelData.type) + " \u00b7 " + row.modelData.key
                                font.family: Theme.mono
                                font.pixelSize: Theme.meta
                                color: Theme.ink3
                            }
                        }

                        Repeater {
                            model: [
                                { glyph: "\u2191", delta: -1, name: "Move up" },
                                { glyph: "\u2193", delta: 1, name: "Move down" }
                            ]

                            delegate: Button {
                                id: mover
                                required property var modelData
                                width: Theme.touchTarget
                                height: Theme.touchTarget
                                // Guarded: a Repeater delegate is created
                                // before it is parented, and reading
                                // `parent.verticalCenter` then is the
                                // TypeError basecamp logged on every field.
                                anchors.verticalCenter: parent ? parent.verticalCenter : undefined
                                text: mover.modelData.glyph
                                Accessible.name: mover.modelData.name + ": " + row.modelData.label
                                onClicked: screen.moveBy(row.index, mover.modelData.delta)
                                contentItem: Text {
                                    text: mover.text
                                    font.pixelSize: Theme.body
                                    color: Theme.ink2
                                    horizontalAlignment: Text.AlignHCenter
                                    verticalAlignment: Text.AlignVCenter
                                }
                                background: Rectangle {
                                    radius: Theme.rControl
                                    color: mover.down ? Theme.line : "transparent"
                                }
                            }
                        }

                        Button {
                            id: removeField
                            width: Theme.touchTarget
                            height: Theme.touchTarget
                            anchors.verticalCenter: parent.verticalCenter
                            text: "\u2715"
                            Accessible.name: "Remove field: " + row.modelData.label
                            onClicked: screen.removeAt(row.index)
                            contentItem: Text {
                                text: removeField.text
                                font.pixelSize: Theme.body
                                color: Theme.ink2
                                horizontalAlignment: Text.AlignHCenter
                                verticalAlignment: Text.AlignVCenter
                            }
                            background: Rectangle {
                                radius: Theme.rControl
                                color: removeField.down ? Theme.line : "transparent"
                            }
                        }
                    }
                }
            }

            // ---- add a field ----
            Text {
                text: "Add a field"
                font.family: Theme.sans
                font.pixelSize: Theme.overline
                font.weight: Theme.semibold
                font.letterSpacing: Theme.overlineSpacing
                font.capitalization: Font.AllUppercase
                color: Theme.ink3
            }

            Flow {
                width: parent.width
                spacing: Theme.s8

                Repeater {
                    model: Kinds.KINDS

                    delegate: Button {
                        id: adder
                        required property var modelData
                        height: Theme.touchTarget
                        leftPadding: Theme.s14
                        rightPadding: Theme.s14
                        text: "+ " + adder.modelData.label
                        Accessible.name: "Add a " + adder.modelData.label + " field. " + adder.modelData.hint
                        onClicked: screen.addField(adder.modelData.type)

                        contentItem: Text {
                            text: adder.text
                            font.family: Theme.sans
                            font.pixelSize: Theme.small
                            color: Theme.accent
                            verticalAlignment: Text.AlignVCenter
                        }
                        background: Rectangle {
                            radius: Theme.rControl
                            color: adder.down ? Theme.accentTint : "transparent"
                            border.width: 1
                            border.color: Theme.accent
                        }
                    }
                }
            }
        }

        // ---- settings for the selected field ----
        FieldSettings {
            visible: screen.selected >= 0
            width: parent.width
            field: screen.selected >= 0 ? screen.fields[screen.selected] : null
            onSetMember: function (member, value) { screen.setOnSelected(member, value); }
            onSetConfig: function (member, value) { screen.setConfigOnSelected(member, value); }
        }

        // ---- submission options ----
        Column {
            width: parent.width
            spacing: Theme.s10

            Text {
                text: "Submissions"
                font.family: Theme.sans
                font.pixelSize: Theme.sectionTitle
                font.weight: Theme.semibold
                color: Theme.ink
            }

            CheckRow {
                id: multipleBox
                width: parent.width
                label: "Allow more than one submission per person"
                on: true
            }

            CheckRow {
                id: draftsBox
                width: parent.width
                label: "Allow drafts"
                hint: "A draft filled in before the end date is still accepted for 7 days after it."
                on: true
            }
        }

        // ---- preview ----
        Column {
            visible: screen.fields.length > 0
            width: parent.width
            spacing: Theme.s12

            Text {
                text: "Preview"
                font.family: Theme.sans
                font.pixelSize: Theme.sectionTitle
                font.weight: Theme.semibold
                color: Theme.ink
            }

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: "This is the real form renderer reading this form's JSON, so it is exactly what a submitter sees."
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
            }

            Rectangle {
                width: parent.width
                implicitHeight: preview.implicitHeight + Theme.s32
                radius: Theme.rPanel
                color: Theme.surfaceSunken

                FormRenderer {
                    id: preview
                    x: Theme.s16
                    y: Theme.s16
                    width: parent.width - Theme.s32
                    protocol: screen.previewProtocol
                }
            }
        }

        // ---- save ----
        Row {
            spacing: Theme.s12

            Button {
                id: saveButton
                height: Theme.touchTarget
                leftPadding: Theme.s28
                rightPadding: Theme.s28
                enabled: screen.complete && !screen.saving
                text: screen.saving ? "Saving\u2026" : "Save form"
                Accessible.name: "Save this form"
                onClicked: {
                    screen.saving = true;
                    screen.save(JSON.stringify(screen.draft()));
                }
                contentItem: Text {
                    text: saveButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: Theme.medium
                    color: saveButton.enabled ? Theme.textOnAccent : Theme.ink3
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: !saveButton.enabled ? Theme.surfaceSunken
                         : (saveButton.down ? Theme.accentHover : Theme.accent)
                }
            }

            Button {
                id: cancelButton
                height: Theme.touchTarget
                leftPadding: Theme.s20
                rightPadding: Theme.s20
                text: "Cancel"
                Accessible.name: "Cancel and go back"
                onClicked: screen.cancelled()
                contentItem: Text {
                    text: cancelButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    color: Theme.ink
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: cancelButton.down ? Theme.surfaceSunken : Theme.surface
                    border.width: 1
                    border.color: Theme.inputBorder
                }
            }
        }

        Text {
            visible: !!screen.problems.fields
            width: parent.width
            wrapMode: Text.Wrap
            text: screen.problems.fields ? screen.problems.fields : ""
            font.family: Theme.sans
            font.pixelSize: Theme.small
            color: Theme.amber
        }

        Text {
            width: parent.width
            wrapMode: Text.Wrap
            text: "Editing a published form saves it as a new version. Submissions already made keep the version they used."
            font.family: Theme.sans
            font.pixelSize: Theme.meta
            color: Theme.ink3
        }
    }
}
