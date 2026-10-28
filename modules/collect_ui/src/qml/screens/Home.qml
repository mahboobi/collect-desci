// C1 — the dashboard.
//
// Every experiment this node knows, as cards, in two sections: the ones this
// identity created and the ones it only submits to. SPEC section 7 calls these
// "Created by you" and "You submit to".
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect
import "../components"

Flickable {
    id: screen

    // Dashboard cards from experiments.list.
    required property var cards

    signal openExperiment(string experimentId, bool mine)
    signal newExperiment()

    readonly property var mine: screen.cards.filter(function (c) { return c.mine === true; })
    readonly property var joined: screen.cards.filter(function (c) { return c.mine !== true; })

    contentWidth: width
    contentHeight: body.implicitHeight + Theme.s48 * 2
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    Column {
        id: body
        x: Theme.s32
        y: Theme.s32
        width: Math.min(screen.width - Theme.s32 * 2, 1040)
        spacing: Theme.s28

        Row {
            width: parent.width
            spacing: Theme.s16

            Column {
                width: parent.width - newButton.width - Theme.s16
                spacing: Theme.s4

                Text {
                    text: "Experiments"
                    font.family: Theme.sans
                    font.pixelSize: Theme.pageTitle
                    font.weight: Theme.semibold
                    font.letterSpacing: Theme.titleSpacing
                    color: Theme.ink
                }

                Text {
                    text: screen.cards.length === 0
                          ? "Nothing yet."
                          : screen.mine.length + " yours \u00b7 " + screen.joined.length + " you can contribute to"
                    font.family: Theme.sans
                    font.pixelSize: Theme.meta
                    color: Theme.ink3
                }
            }

            Button {
                id: newButton
                anchors.verticalCenter: parent.verticalCenter
                height: Theme.touchTarget
                leftPadding: Theme.s24
                rightPadding: Theme.s24
                text: "New experiment"
                Accessible.name: "Create a new experiment"
                onClicked: screen.newExperiment()

                contentItem: Text {
                    text: newButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: Theme.medium
                    color: Theme.textOnAccent
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: newButton.down ? Theme.accentHover : Theme.accent
                }
            }
        }

        // ---- the two sections ----
        Repeater {
            model: [
                { key: "mine",   heading: "Created by you", empty: "You have not created an experiment yet." },
                { key: "joined", heading: "You can contribute to", empty: "No experiments from anyone else on this node yet." }
            ]

            delegate: Column {
                id: section
                required property var modelData
                readonly property var items: section.modelData.key === "mine" ? screen.mine : screen.joined

                width: body.width
                spacing: Theme.s12

                Text {
                    text: section.modelData.heading
                    font.family: Theme.sans
                    font.pixelSize: Theme.overline
                    font.weight: Theme.semibold
                    font.letterSpacing: Theme.overlineSpacing
                    font.capitalization: Font.AllUppercase
                    color: Theme.ink3
                }

                Text {
                    visible: section.items.length === 0
                    width: section.width
                    wrapMode: Text.Wrap
                    text: section.modelData.empty
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    color: Theme.ink3
                }

                // Cards wrap into as many columns as the width allows.
                Flow {
                    width: section.width
                    spacing: Theme.s16

                    Repeater {
                        model: section.items

                        delegate: ExperimentCard {
                            required property var modelData
                            model: modelData
                            width: Math.min(section.width, Math.max(320, (section.width - Theme.s16 * 2) / 3))
                            onOpened: screen.openExperiment(modelData.id, modelData.mine === true)
                        }
                    }
                }
            }
        }
    }
}
