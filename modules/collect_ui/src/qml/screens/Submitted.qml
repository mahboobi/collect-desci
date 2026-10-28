// S3 — what happened to the submission.
//
// Four steps, because they fail independently and a submitter who sees
// "submitted" has not been told whether the creator actually has it. The
// fourth stays "Waiting" until a receipt arrives, which is the honest state:
// Delivery Store is temporary and the creator may be offline.
import QtQuick
import QtQuick.Controls
import collect
import "../components"

Item {
    id: screen

    required property var protocol
    property string cid: ""
    property bool received: false

    signal submitAnother()
    signal backToExperiment()

    Column {
        x: Theme.s32
        y: Theme.s32
        width: Math.min(screen.width - Theme.s32 * 2, 640)
        spacing: Theme.s24

        Column {
            width: parent.width
            spacing: Theme.s8

            Text {
                text: "Submitted"
                font.family: Theme.sans
                font.pixelSize: Theme.pageTitle
                font.weight: Theme.semibold
                font.letterSpacing: Theme.titleSpacing
                color: Theme.ink
            }

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: "Thank you. Your submission is saved."
                font.family: Theme.sans
                font.pixelSize: Theme.lede
                color: Theme.ink2
            }
        }

        Column {
            width: parent.width
            spacing: Theme.s12

            Repeater {
                model: [
                    {
                        label: "Signed with your Basecamp key",
                        detail: "",
                        done: true
                    },
                    {
                        label: "Saved on Logos Storage",
                        detail: Core.shortCid(screen.cid),
                        done: screen.cid !== ""
                    },
                    {
                        label: "Announced to the experiment over Logos Delivery",
                        detail: "",
                        done: screen.cid !== ""
                    },
                    {
                        label: "Received by the creator's node",
                        detail: "",
                        done: screen.received
                    }
                ]

                delegate: Row {
                    id: step
                    required property var modelData

                    width: parent.width
                    spacing: Theme.s12

                    Rectangle {
                        width: Theme.statusDot
                        height: Theme.statusDot
                        radius: width / 2
                        anchors.verticalCenter: parent.verticalCenter
                        color: step.modelData.done ? Theme.accent : Theme.amber
                    }

                    Column {
                        anchors.verticalCenter: parent.verticalCenter
                        width: step.width - Theme.statusDot - stepBadge.width - Theme.s24
                        spacing: Theme.s2

                        Text {
                            text: step.modelData.label
                            font.family: Theme.sans
                            font.pixelSize: Theme.body
                            color: Theme.ink
                            elide: Text.ElideRight
                            width: parent.width
                        }

                        Text {
                            visible: step.modelData.detail !== ""
                            text: step.modelData.detail
                            font.family: Theme.mono
                            font.pixelSize: Theme.meta
                            color: Theme.ink3
                        }
                    }

                    Badge {
                        id: stepBadge
                        anchors.verticalCenter: parent.verticalCenter
                        text: step.modelData.done ? "Done" : "Waiting"
                        variant: step.modelData.done ? "done" : "attention"
                    }
                }
            }
        }

        // Explains why step four can sit on Waiting, so it does not read as a
        // failure.
        Text {
            visible: !screen.received
            width: parent.width
            wrapMode: Text.Wrap
            text: "Keep Basecamp open for a few minutes. If the creator's node is offline, your Basecamp keeps announcing the submission until it is received."
            font.family: Theme.sans
            font.pixelSize: Theme.small
            color: Theme.ink2
            lineHeight: Theme.bodyLineHeight
            lineHeightMode: Text.ProportionalHeight
        }

        Row {
            spacing: Theme.s12

            Button {
                id: againButton
                visible: !!(screen.protocol && screen.protocol.allow_multiple)
                height: Theme.touchTarget
                leftPadding: Theme.s20
                rightPadding: Theme.s20
                text: "Submit another"
                Accessible.name: "Submit another response to this form"
                onClicked: screen.submitAnother()
                contentItem: Text {
                    text: againButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: Theme.medium
                    color: Theme.textOnAccent
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: againButton.down ? Theme.accentHover : Theme.accent
                }
            }

            Button {
                id: backButton
                height: Theme.touchTarget
                leftPadding: Theme.s20
                rightPadding: Theme.s20
                text: "Back to the experiment"
                Accessible.name: "Back to the experiment"
                onClicked: screen.backToExperiment()
                contentItem: Text {
                    text: backButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    color: Theme.ink
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: backButton.down ? Theme.surfaceSunken : Theme.surface
                    border.width: 1
                    border.color: Theme.inputBorder
                }
            }
        }
    }
}
