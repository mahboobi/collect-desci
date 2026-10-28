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

        Text {
            width: parent.width
            wrapMode: Text.Wrap
            text: screen.experiment ? screen.experiment.title : ""
            font.family: Theme.sans
            font.pixelSize: Theme.pageTitle
            font.weight: Theme.semibold
            font.letterSpacing: Theme.titleSpacing
            color: Theme.ink
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

        // ---- the forms ----
        Column {
            width: parent.width
            spacing: Theme.s12

            Text {
                text: "Forms"
                font.family: Theme.sans
                font.pixelSize: Theme.sectionTitle
                font.weight: Theme.semibold
                color: Theme.ink
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
                            width: parent.width - fillButton.width - Theme.s24
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
                            id: fillButton
                            anchors.verticalCenter: parent.verticalCenter
                            height: Theme.touchTarget
                            leftPadding: Theme.s20
                            rightPadding: Theme.s20
                            enabled: card.fillable
                            text: card.fillable ? "Fill in" : "Not accepting"
                            Accessible.name: (card.doc ? card.doc.name : card.modelData.id)
                                             + (card.fillable ? ": fill in this form" : ": not accepting submissions")
                            onClicked: screen.openProtocol(card.modelData.id)

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
