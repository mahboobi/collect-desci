// S1 — the experiment as a submitter sees it after opening a link.
//
// Everything a reader needs to decide whether to trust and take part: who made
// it, the key fingerprint, the dates, the public-data warning, and the forms.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect
import "../components"

Flickable {
    id: screen

    required property var experiment
    // Millisecond clock, injected so a test can place itself in or out of the
    // window instead of depending on the real date.
    property double nowMs: Date.now()

    readonly property bool open: Core.withinWindow(experiment, nowMs)

    signal openProtocol(string protocolId)
    signal editProtocol(string protocolId)
    signal openResults(string protocolId)
    signal addProtocol()
    signal editExperiment()
    signal back()

    // The forms are behind an explicit step. Someone opening a link is
    // reading first, and the public-data notice above is the thing they need
    // to have seen before a form appears.
    property bool contributing: false

    // A creator looking at their own experiment is editing, not contributing.
    property bool mine: false

    contentWidth: width
    contentHeight: body.implicitHeight + Theme.s48 * 2
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    Column {
        id: body
        x: Theme.s32
        y: Theme.s32
        width: Math.min(screen.width - Theme.s32 * 2, 760)
        spacing: Theme.s24

        Button {
            id: backButton
            height: Theme.chipHeight
            leftPadding: Theme.s12
            rightPadding: Theme.s14
            text: "\u2190 All experiments"
            Accessible.name: "Back to all experiments"
            onClicked: screen.back()
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

        Row {
            width: parent.width
            spacing: Theme.s12

            Text {
                width: parent.width - (editDetails.visible ? editDetails.width + Theme.s12 : 0)
                wrapMode: Text.Wrap
                text: screen.experiment ? screen.experiment.title : ""
                font.family: Theme.sans
                font.pixelSize: Theme.pageTitle
                font.weight: Theme.semibold
                font.letterSpacing: Theme.titleSpacing
                color: Theme.ink
            }

            Button {
                id: editDetails
                visible: screen.mine
                anchors.top: parent.top
                height: Theme.chipHeight
                leftPadding: Theme.s14
                rightPadding: Theme.s14
                text: "Edit details"
                Accessible.name: "Edit this experiment's details"
                onClicked: screen.editExperiment()
                contentItem: Text {
                    text: editDetails.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    font.weight: Theme.medium
                    color: Theme.accent
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rPill
                    color: editDetails.down ? Theme.surfaceSunken : "transparent"
                    border.width: 1
                    border.color: Theme.line
                }
            }
        }

        // ---- who, and when ----
        Column {
            width: parent.width
            spacing: Theme.s6

            Row {
                spacing: Theme.s8

                Text {
                    text: screen.experiment && screen.experiment.creator && screen.experiment.creator.name
                          ? screen.experiment.creator.name
                          : "Unnamed creator"
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: Theme.medium
                    color: Theme.ink
                }

                Badge {
                    text: screen.open ? "Open" : "Closed"
                    variant: screen.open ? "done" : "neutral"
                }
            }

            // The fingerprint, not the key: 64 hex characters is noise, and
            // the point is to let someone compare it out of band.
            Text {
                text: "Key " + (screen.experiment && screen.experiment.creator
                                ? Core.fingerprint(screen.experiment.creator.key) : "")
                font.family: Theme.mono
                font.pixelSize: Theme.fingerprint
                font.letterSpacing: 0.56
                color: Theme.ink2
            }

            Text {
                text: screen.experiment
                      ? "Open from " + screen.experiment.starts_at.slice(0, 10)
                        + " to " + screen.experiment.ends_at.slice(0, 10)
                        + " \u00b7 " + screen.experiment.timezone
                      : ""
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
            }
        }

        Text {
            visible: !!(screen.experiment && screen.experiment.description)
            width: parent.width
            wrapMode: Text.Wrap
            text: screen.experiment ? screen.experiment.description : ""
            font.family: Theme.sans
            font.pixelSize: Theme.lede
            color: Theme.ink2
            lineHeight: Theme.bodyLineHeight
            lineHeightMode: Text.ProportionalHeight
        }

        PublicNotice {}

        // ---- the gate ----
        Column {
            visible: !screen.contributing
            width: parent.width
            spacing: Theme.s12

            Button {
                id: contributeButton
                height: Theme.touchTarget
                leftPadding: Theme.s28
                rightPadding: Theme.s28
                enabled: screen.mine || (screen.open && Core.protocolsOf(screen.experiment).length > 0)
                text: screen.mine ? "Open the forms" : "I would like to contribute to this experiment"
                Accessible.name: contributeButton.text
                onClicked: screen.contributing = true

                contentItem: Text {
                    text: contributeButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: Theme.medium
                    color: contributeButton.enabled ? Theme.textOnAccent : Theme.ink3
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: !contributeButton.enabled ? Theme.surfaceSunken
                         : (contributeButton.down ? Theme.accentHover : Theme.accent)
                }
            }

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: !screen.open
                      ? "This experiment is not accepting submissions right now."
                      : (Core.protocolsOf(screen.experiment).length === 0
                         ? "This experiment has no forms yet."
                         : (screen.mine ? "" : "You will see the forms after this, and nothing is submitted until you press Submit."))
                visible: text !== ""
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: screen.open ? Theme.ink3 : Theme.amber
            }
        }

        // ---- the forms ----
        Column {
            visible: screen.contributing
            width: parent.width
            spacing: Theme.s12

            Row {
                width: parent.width
                spacing: Theme.s12

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Forms"
                    font.family: Theme.sans
                    font.pixelSize: Theme.sectionTitle
                    font.weight: Theme.semibold
                    color: Theme.ink
                }

                Button {
                    id: addForm
                    visible: screen.mine
                    anchors.verticalCenter: parent.verticalCenter
                    height: Theme.chipHeight
                    leftPadding: Theme.s14
                    rightPadding: Theme.s14
                    text: "+ Add a form"
                    Accessible.name: "Add a form to this experiment"
                    onClicked: screen.addProtocol()
                    contentItem: Text {
                        text: addForm.text
                        font.family: Theme.sans
                        font.pixelSize: Theme.small
                        font.weight: Theme.medium
                        color: Theme.accent
                        verticalAlignment: Text.AlignVCenter
                    }
                    background: Rectangle {
                        radius: Theme.rPill
                        color: addForm.down ? Theme.accentTint : "transparent"
                        border.width: 1
                        border.color: Theme.accent
                    }
                }
            }

            Repeater {
                model: Core.protocolsOf(screen.experiment)

                delegate: Rectangle {
                    id: card
                    required property var modelData

                    readonly property var doc: Core.protocolById(modelData.id)
                    // A protocol is fillable only when it is open AND the
                    // experiment is inside its dates.
                    readonly property bool fillable: modelData.status === "open" && screen.open

                    width: body.width
                    implicitHeight: Theme.touchTarget + Theme.s32
                    radius: Theme.rPanel
                    color: Theme.surface
                    border.width: 1
                    border.color: Theme.line

                    Row {
                        anchors.fill: parent
                        anchors.margins: Theme.s16
                        spacing: Theme.s12

                        Column {
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - fillButton.width
                                   - (screen.mine ? resultsButton.width + Theme.s12 : 0) - Theme.s24
                            spacing: Theme.s4

                            Text {
                                text: card.doc ? card.doc.name : card.modelData.id
                                font.family: Theme.sans
                                font.pixelSize: Theme.body
                                font.weight: Theme.medium
                                color: Theme.ink
                                elide: Text.ElideRight
                                width: parent.width
                            }

                            Text {
                                text: "Version " + card.modelData.version
                                      + " \u00b7 " + (card.modelData.status === "open" ? "Open" : "Closed")
                                font.family: Theme.sans
                                font.pixelSize: Theme.meta
                                color: Theme.ink3
                            }
                        }

                        Button {
                            id: resultsButton
                            visible: screen.mine
                            anchors.verticalCenter: parent.verticalCenter
                            height: Theme.touchTarget
                            leftPadding: Theme.s16
                            rightPadding: Theme.s16
                            text: "Results"
                            Accessible.name: "See results for " + (card.doc ? card.doc.name : card.modelData.id)
                            onClicked: screen.openResults(card.modelData.id)
                            contentItem: Text {
                                text: resultsButton.text
                                font.family: Theme.sans
                                font.pixelSize: Theme.small
                                font.weight: Theme.medium
                                color: Theme.accent
                                verticalAlignment: Text.AlignVCenter
                            }
                            background: Rectangle {
                                radius: Theme.rControl
                                color: resultsButton.down ? Theme.accentTint : "transparent"
                                border.width: 1
                                border.color: Theme.accent
                            }
                        }

                        Button {
                            id: fillButton
                            anchors.verticalCenter: parent.verticalCenter
                            height: Theme.touchTarget
                            leftPadding: Theme.s20
                            rightPadding: Theme.s20
                            enabled: card.fillable
                            text: !card.fillable ? "Not accepting" : (screen.mine ? "Edit" : "Fill in")
                            Accessible.name: (card.doc ? card.doc.name : card.modelData.id)
                                             + (!card.fillable ? ": not accepting submissions"
                                                : (screen.mine ? ": edit this form" : ": fill in this form"))
                            onClicked: screen.mine ? screen.editProtocol(card.modelData.id)
                                                   : screen.openProtocol(card.modelData.id)

                            contentItem: Text {
                                text: fillButton.text
                                font.family: Theme.sans
                                font.pixelSize: Theme.small
                                font.weight: Theme.medium
                                color: fillButton.enabled ? Theme.textOnAccent : Theme.ink3
                                horizontalAlignment: Text.AlignHCenter
                                verticalAlignment: Text.AlignVCenter
                            }
                            background: Rectangle {
                                radius: Theme.rControl
                                color: !fillButton.enabled ? Theme.surfaceSunken
                                     : (fillButton.down ? Theme.accentHover : Theme.accent)
                            }
                        }
                    }
                }
            }
        }
    }
}
