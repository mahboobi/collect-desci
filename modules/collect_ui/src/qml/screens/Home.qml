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

    // delivery.status's `connection`: { level: online | degraded | offline,
    // reason }. Offline until the first answer, which is what it is.
    property var connection: ({ level: "offline", reason: "Not checked yet." })
    // A refresh is in flight: the icon spins and the button waits.
    property bool refreshing: false

    signal openExperiment(string experimentId, bool mine)
    signal newExperiment()
    signal refresh()
    /// Join someone else's experiment from its share link.
    signal joinLink(string link)
    property bool joinBusy: false
    property string joinError: ""
    // What is typed (LabelledInput's `text` only flows inward).
    property string typedLink: ""

    readonly property string connectionLevel: screen.connection && screen.connection.level
                                              ? screen.connection.level : "offline"
    readonly property string connectionLabel: screen.connectionLevel === "online" ? "Online"
                                              : screen.connectionLevel === "degraded" ? "Weak connection"
                                              : "Offline"
    readonly property color connectionColor: screen.connectionLevel === "online" ? Theme.statusOnline
                                             : screen.connectionLevel === "degraded" ? Theme.statusDegraded
                                             : Theme.statusOffline

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
                width: parent.width - actions.width - Theme.s16
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

            Row {
                id: actions
                anchors.verticalCenter: parent.verticalCenter
                spacing: Theme.s12

                // The connection dot. Focusable so the reason is reachable from
                // the keyboard as well as by hovering.
                Rectangle {
                    id: status
                    anchors.verticalCenter: parent.verticalCenter
                    height: Theme.touchTarget
                    width: statusRow.implicitWidth + Theme.s24
                    radius: Theme.rControl
                    color: statusArea.containsMouse || status.activeFocus ? Theme.surfaceSunken : "transparent"
                    activeFocusOnTab: true
                    Accessible.role: Accessible.StaticText
                    Accessible.name: "Connection: " + screen.connectionLabel + ". "
                                     + (screen.connection && screen.connection.reason ? screen.connection.reason : "")

                    Row {
                        id: statusRow
                        anchors.centerIn: parent
                        spacing: Theme.s8

                        Rectangle {
                            objectName: "connectionDot"
                            anchors.verticalCenter: parent.verticalCenter
                            width: 10
                            height: 10
                            radius: 5
                            color: screen.connectionColor
                        }

                        Text {
                            objectName: "connectionLabel"
                            anchors.verticalCenter: parent.verticalCenter
                            text: screen.connectionLabel
                            font.family: Theme.sans
                            font.pixelSize: Theme.small
                            color: Theme.ink2
                        }
                    }

                    MouseArea {
                        id: statusArea
                        anchors.fill: parent
                        hoverEnabled: true
                    }

                    ToolTip.visible: statusArea.containsMouse || status.activeFocus
                    ToolTip.delay: 300
                    ToolTip.text: screen.connection && screen.connection.reason ? screen.connection.reason : ""
                }

                Button {
                    id: refreshButton
                    objectName: "refreshButton"
                    anchors.verticalCenter: parent.verticalCenter
                    width: Theme.touchTarget
                    height: Theme.touchTarget
                    enabled: !screen.refreshing
                    Accessible.name: screen.refreshing ? "Refreshing" : "Refresh experiments and connection"
                    ToolTip.visible: hovered
                    ToolTip.delay: 300
                    ToolTip.text: "Refresh"
                    onClicked: screen.refresh()

                    // Drawn rather than a glyph: the bundled fonts carry no
                    // circular arrow, and a missing glyph renders as a box.
                    contentItem: Canvas {
                        id: arrow
                        width: 20
                        height: 20
                        property color ink: Theme.ink2
                        onInkChanged: requestPaint()
                        onPaint: {
                            var c = getContext("2d");
                            c.reset();
                            c.strokeStyle = arrow.ink;
                            c.fillStyle = arrow.ink;
                            c.lineWidth = 2;
                            c.lineCap = "round";
                            var cx = width / 2, cy = height / 2, r = 7;
                            c.beginPath();
                            c.arc(cx, cy, r, -Math.PI * 0.35, Math.PI * 1.45);
                            c.stroke();
                            // Arrowhead at the arc's open end.
                            var a = -Math.PI * 0.35;
                            var ex = cx + r * Math.cos(a), ey = cy + r * Math.sin(a);
                            c.beginPath();
                            c.moveTo(ex + 4, ey - 1);
                            c.lineTo(ex - 1, ey - 4);
                            c.lineTo(ex - 1, ey + 3);
                            c.closePath();
                            c.fill();
                        }

                        RotationAnimator on rotation {
                            running: screen.refreshing
                            from: 0
                            to: 360
                            duration: 900
                            loops: Animation.Infinite
                            onRunningChanged: if (!running) arrow.rotation = 0
                        }
                    }
                    background: Rectangle {
                        radius: Theme.rControl
                        color: refreshButton.down ? Theme.surfaceSunken : Theme.surface
                        border.color: refreshButton.activeFocus ? Theme.accent : Theme.line
                        border.width: 1
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
        }

        // ---- join with a link ----
        Column {
            width: parent.width
            spacing: Theme.s6

            Row {
                width: parent.width
                spacing: Theme.s12

                LabelledInput {
                    id: linkInput
                    objectName: "joinLinkInput"
                    width: parent.width - joinButton.width - Theme.s12
                    label: "Join with a link"
                    mono: true
                    hint: screen.joinError
                    onEdited: function (value) { screen.typedLink = value.trim(); }
                }

                Button {
                    id: joinButton
                    objectName: "joinButton"
                    anchors.bottom: parent.bottom
                    anchors.bottomMargin: linkInput.hint !== "" ? Theme.s24 : 0
                    height: Theme.touchTarget
                    leftPadding: Theme.s20
                    rightPadding: Theme.s20
                    enabled: !screen.joinBusy && screen.typedLink !== ""
                    text: screen.joinBusy ? "Fetching\u2026" : "Join"
                    Accessible.name: "Join the experiment at this link"
                    onClicked: screen.joinLink(screen.typedLink)
                    contentItem: Text {
                        text: joinButton.text
                        font.family: Theme.sans
                        font.pixelSize: Theme.body
                        font.weight: Theme.medium
                        color: joinButton.enabled ? Theme.accent : Theme.ink3
                        verticalAlignment: Text.AlignVCenter
                    }
                    background: Rectangle {
                        radius: Theme.rControl
                        color: joinButton.down ? Theme.accentTint : Theme.surface
                        border.width: 1
                        border.color: joinButton.enabled ? Theme.accent : Theme.line
                    }
                }
            }

            Text {
                visible: screen.joinError === ""
                width: parent.width
                wrapMode: Text.Wrap
                text: "Paste a link someone shared (it starts with logos-collect://). Collect fetches the experiment "
                      + "and its forms from Logos Storage and checks they are signed by its creator."
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
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
