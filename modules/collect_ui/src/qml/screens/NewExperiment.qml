// C2 — experiment details.
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

    signal save(string json)
    signal cancelled()

    readonly property bool datesOrdered: startField.stamp === "" || endField.stamp === ""
                                         || Date.parse(endField.stamp) >= Date.parse(startField.stamp)
    readonly property bool complete: titleField.text.trim() !== ""
                                     && startField.stamp !== "" && endField.stamp !== ""
                                     && datesOrdered

    contentWidth: width
    contentHeight: body.implicitHeight + Theme.s48 * 2
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    function problemFor(field) {
        return screen.problems && screen.problems[field] ? screen.problems[field] : "";
    }

    Column {
        id: body
        x: Theme.s32
        y: Theme.s32
        width: Math.min(screen.width - Theme.s32 * 2, 720)
        spacing: Theme.s24

        Text {
            text: "New experiment"
            font.family: Theme.sans
            font.pixelSize: Theme.pageTitle
            font.weight: Theme.semibold
            font.letterSpacing: Theme.titleSpacing
            color: Theme.ink
        }

        PublicNotice {
            detail: "Everything in this experiment, including every submission, is stored publicly on Logos Storage and cannot be deleted."
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

            Repeater {
                model: [
                    { key: "startsAt", label: "Starts" },
                    { key: "endsAt", label: "Ends" }
                ]

                delegate: Column {
                    id: dateColumn
                    required property var modelData
                    spacing: Theme.s8

                    Text {
                        text: dateColumn.modelData.label + " \u00b7 required"
                        font.family: Theme.sans
                        font.pixelSize: Theme.meta
                        font.weight: Theme.medium
                        color: Theme.ink2
                    }

                    Rectangle {
                        width: 240
                        height: Theme.touchTarget
                        radius: Theme.rControl
                        color: Theme.surface
                        border.width: 1
                        border.color: screen.problemFor(dateColumn.modelData.key) !== "" || !screen.datesOrdered
                                      ? Theme.amber : Theme.inputBorder

                        TextInput {
                            id: dateInput
                            objectName: dateColumn.modelData.key
                            anchors.fill: parent
                            anchors.margins: Theme.s10
                            verticalAlignment: TextInput.AlignVCenter
                            font.family: Theme.mono
                            font.pixelSize: Theme.body
                            color: Theme.ink
                            selectByMouse: true
                            Accessible.name: dateColumn.modelData.label + " date and time"
                            Accessible.description: "RFC 3339 with an offset, for example 2026-11-01T00:00:00+05:30"
                            onTextEdited: {
                                if (dateColumn.modelData.key === "startsAt")
                                    startField.stamp = text;
                                else
                                    endField.stamp = text;
                            }
                        }
                    }
                }
            }
        }

        // Two holders, so the bindings above have somewhere stable to write.
        QtObject { id: startField; property string stamp: "" }
        QtObject { id: endField; property string stamp: "" }

        Text {
            visible: !screen.datesOrdered || screen.problemFor("endsAt") !== ""
            width: parent.width
            wrapMode: Text.Wrap
            text: !screen.datesOrdered
                  ? "The end date cannot be before the start date."
                  : screen.problemFor("endsAt")
            font.family: Theme.sans
            font.pixelSize: Theme.small
            color: Theme.amber
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
                text: screen.saving ? "Saving\u2026" : "Create experiment"
                Accessible.name: "Create this experiment"
                onClicked: {
                    screen.saving = true;
                    screen.save(JSON.stringify({
                        title: titleField.text.trim(),
                        description: descriptionField.text,
                        startsAt: startField.stamp,
                        endsAt: endField.stamp,
                        timezone: timezoneField.text.trim()
                    }));
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
