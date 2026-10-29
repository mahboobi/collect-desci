// C2 — experiment details, for a new experiment and for editing one.
//
// One screen for both, because the fields and every rule over them are the
// same: a title length accepted on create and refused on edit would be a bug
// only an editing user ever hits. `existing` switches the wording and
// prefills; which method the payload goes to is the router's business.
//
// SPEC 3.1: title, description, start and end date, time zone, and the end
// must not be before the start — inline error, Save disabled.
//
// The dates are checked here for immediate feedback and again in
// collect_core, which is authoritative and returns per-field problems.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect
import "../components"

Flickable {
    id: screen

    // Per-field complaints from collect_core, keyed by field name.
    property var problems: ({})
    property bool saving: false
    /// The experiment being edited, or null when creating one.
    property var existing: null
    /// Warnings collect_core returned about the edit — notably that the new
    /// dates exclude submissions it has already accepted.
    property var warnings: []

    readonly property bool editing: screen.existing !== null

    signal save(string json)
    signal cancelled()

    readonly property bool datesOrdered: startPicker.stamp === "" || endPicker.stamp === ""
                                         || Date.parse(endPicker.stamp) >= Date.parse(startPicker.stamp)
    readonly property bool complete: titleField.text.trim() !== ""
                                     && startPicker.stamp !== "" && endPicker.stamp !== ""
                                     && datesOrdered

    contentWidth: width
    contentHeight: body.implicitHeight + Theme.s48 * 2
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    function problemFor(field) {
        return screen.problems && screen.problems[field] ? screen.problems[field] : "";
    }

    /// Fill the form from `existing`. Called once, on load: binding the
    /// fields to it instead would make them read-only in practice, since
    /// every keystroke would be overwritten by the binding.
    function prefill() {
        if (!screen.existing)
            return;
        titleField.text = screen.existing.title ? screen.existing.title : "";
        descriptionField.text = screen.existing.description ? screen.existing.description : "";
        // `stamp` is derived; the day is what the picker owns. Taking the
        // first ten characters is the whole date — the stored value is always
        // RFC 3339, so the offset and time are rebuilt from `endOfDay`.
        startPicker.isoDate = screen.existing.starts_at
                              ? screen.existing.starts_at.slice(0, 10) : "";
        endPicker.isoDate = screen.existing.ends_at
                            ? screen.existing.ends_at.slice(0, 10) : "";
        timezoneField.text = screen.existing.timezone ? screen.existing.timezone : "";
    }

    Component.onCompleted: screen.prefill()

    Column {
        id: body
        x: Theme.s32
        y: Theme.s32
        width: Math.min(screen.width - Theme.s32 * 2, 720)
        spacing: Theme.s24

        // The same back control the other screens have. Cancel sits at the
        // bottom of a long form, which is no use to someone who has scrolled
        // and changed their mind.
        Button {
            id: backButton
            height: Theme.chipHeight
            leftPadding: Theme.s12
            rightPadding: Theme.s14
            text: screen.editing ? "\u2190 Back to the experiment" : "\u2190 All experiments"
            Accessible.name: screen.editing ? "Back to the experiment" : "Back to all experiments"
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
            text: screen.editing ? "Edit experiment" : "New experiment"
            font.family: Theme.sans
            font.pixelSize: Theme.pageTitle
            font.weight: Theme.semibold
            font.letterSpacing: Theme.titleSpacing
            color: Theme.ink
        }

        PublicNotice {
            detail: screen.editing
                    ? "Saving publishes a new version of this experiment. The version it replaces stays on Logos Storage and cannot be deleted."
                    : "Everything in this experiment, including every submission, is stored publicly on Logos Storage and cannot be deleted."
        }

        // What the edit would do to submissions already accepted. Shown
        // after the fact because only collect_core knows what it holds, and
        // it warns rather than refusing: those submissions were valid when
        // they were made.
        Column {
            visible: screen.warnings.length > 0
            width: parent.width
            spacing: Theme.s4

            Repeater {
                model: screen.warnings

                delegate: Text {
                    id: warningLine
                    required property var modelData
                    width: screen.width - Theme.s32 * 2
                    wrapMode: Text.Wrap
                    text: warningLine.modelData.message
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    color: Theme.amber
                }
            }
        }

        // ---- title ----
        Column {
            width: parent.width
            spacing: Theme.s8

            Text {
                text: "Title \u00b7 required"
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
                border.color: titleField.activeFocus ? Theme.accent
                            : (screen.problemFor("title") !== "" ? Theme.amber : Theme.inputBorder)

                TextInput {
                    id: titleField
                    anchors.fill: parent
                    anchors.margins: Theme.s10
                    verticalAlignment: TextInput.AlignVCenter
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    color: Theme.ink
                    selectByMouse: true
                    maximumLength: 140
                    Accessible.name: "Experiment title"
                }
            }

            Text {
                visible: screen.problemFor("title") !== ""
                text: screen.problemFor("title")
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: Theme.amber
            }
        }

        // ---- description ----
        Column {
            width: parent.width
            spacing: Theme.s8

            Text {
                text: "Description \u00b7 optional"
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                font.weight: Theme.medium
                color: Theme.ink2
            }

            Rectangle {
                width: parent.width
                height: 112
                radius: Theme.rControl
                color: Theme.surface
                border.width: 1
                border.color: descriptionField.activeFocus ? Theme.accent : Theme.inputBorder

                ScrollView {
                    anchors.fill: parent
                    anchors.margins: Theme.s10
                    clip: true

                    TextArea {
                        id: descriptionField
                        wrapMode: TextEdit.Wrap
                        font.family: Theme.sans
                        font.pixelSize: Theme.body
                        color: Theme.ink
                        background: null
                        padding: 0
                        selectByMouse: true
                        Accessible.name: "Experiment description"
                    }
                }
            }
        }

        // ---- dates ----
        Row {
            spacing: Theme.s16

            DatePicker {
                id: startPicker
                label: "Starts \u00b7 required"
                width: 260
                invalid: screen.problemFor("startsAt") !== "" || !screen.datesOrdered
            }

            DatePicker {
                id: endPicker
                label: "Ends \u00b7 required"
                width: 260
                // An end date means the end of that day, not its first
                // instant — otherwise picking today would close the
                // experiment before it opened.
                endOfDay: true
                invalid: screen.problemFor("endsAt") !== "" || !screen.datesOrdered
            }
        }

        // Each field's complaint, plus the ordering rule. Folding these into
        // one line hid any startsAt problem entirely.
        Column {
            width: parent.width
            spacing: Theme.s4

            Repeater {
                model: [
                    { key: "startsAt", label: "Starts" },
                    { key: "endsAt", label: "Ends" }
                ]

                delegate: Text {
                    id: problemLine
                    required property var modelData
                    visible: screen.problemFor(problemLine.modelData.key) !== ""
                    width: parent.width
                    wrapMode: Text.Wrap
                    text: problemLine.modelData.label + ": " + screen.problemFor(problemLine.modelData.key)
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    color: Theme.amber
                }
            }

            Text {
                visible: !screen.datesOrdered
                width: parent.width
                wrapMode: Text.Wrap
                text: "The end date cannot be before the start date."
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: Theme.amber
            }
        }

        // ---- time zone ----
        Column {
            width: parent.width
            spacing: Theme.s8

            Text {
                text: "Time zone"
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                font.weight: Theme.medium
                color: Theme.ink2
            }

            Rectangle {
                width: 320
                height: Theme.touchTarget
                radius: Theme.rControl
                color: Theme.surface
                border.width: 1
                border.color: Theme.inputBorder

                TextInput {
                    id: timezoneField
                    anchors.fill: parent
                    anchors.margins: Theme.s10
                    verticalAlignment: TextInput.AlignVCenter
                    text: "UTC"
                    font.family: Theme.mono
                    font.pixelSize: Theme.body
                    color: Theme.ink
                    selectByMouse: true
                    Accessible.name: "Time zone, an IANA name such as Asia/Kolkata"
                }
            }
        }

        Row {
            spacing: Theme.s12

            Button {
                id: saveButton
                height: Theme.touchTarget
                leftPadding: Theme.s28
                rightPadding: Theme.s28
                enabled: screen.complete && !screen.saving
                text: screen.saving
                      ? "Saving\u2026"
                      : (screen.editing ? "Save changes" : "Create experiment")
                Accessible.name: screen.editing
                                 ? "Save changes to this experiment"
                                 : "Create this experiment"
                onClicked: {
                    screen.saving = true;
                    var payload = {
                        title: titleField.text.trim(),
                        description: descriptionField.text,
                        startsAt: startPicker.stamp,
                        endsAt: endPicker.stamp,
                        timezone: timezoneField.text.trim()
                    };
                    if (screen.editing) {
                        payload.experimentId = screen.existing.id;
                        // The creator's own display name travels with the
                        // edit; the key does not, and collect_core refuses to
                        // change it regardless of what is sent.
                        payload.creatorName = screen.existing.creator
                                              ? screen.existing.creator.name : "";
                    }
                    screen.save(JSON.stringify(payload));
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
            width: parent.width
            wrapMode: Text.Wrap
            text: "Saved on this node only. Publishing to Logos Storage is not wired yet, so nobody else can open it."
            font.family: Theme.sans
            font.pixelSize: Theme.meta
            color: Theme.ink3
        }
    }
}
