// Discussion — the members of one project talking about it.
//
// Messages go over the experiment's Logos Delivery topic and are kept on each
// member's device; they are never written to Logos Storage. They are readable
// by anyone who has the experiment's link, which the screen says plainly.
// Names and photos appear only for members who shared them with us.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect
import "../components"

Item {
    id: screen

    required property var experiment
    /// `messages.list` data.
    property var thread: null
    /// `members.list` data.
    property var people: null
    property bool sending: false
    /// The last send failure, shown above the composer.
    property string sendError: ""

    signal back()
    signal send(string text)
    signal refresh()

    readonly property var messages: thread && thread.messages ? thread.messages : []
    readonly property var members: people && people.members ? people.members : []
    readonly property bool canSend: !!(thread && thread.can_send)
    readonly property bool rlnBlocked: !!(thread && thread.rln && thread.rln.required && !thread.rln.ready)

    /// The member whose shared profile is open, or null.
    property var openMember: null

    function label(m) {
        if (m.mine)
            return m.name ? m.name + " (you)" : "You";
        return m.name ? m.name : m.short + "…";
    }

    function when(iso) {
        var d = new Date(iso);
        if (isNaN(d.getTime()))
            return "";
        return d.toLocaleString(Qt.locale(), "d MMM, HH:mm");
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.ground
    }

    // ---- header ----
    Column {
        id: header
        x: Theme.s32
        y: Theme.s24
        width: parent.width - Theme.s32 * 2
        spacing: Theme.s12

        Button {
            id: backButton
            height: Theme.chipHeight
            leftPadding: Theme.s12
            rightPadding: Theme.s14
            text: "← Back to the experiment"
            Accessible.name: "Back to the experiment"
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

        Text {
            width: parent.width
            elide: Text.ElideRight
            text: "Discussion · " + (screen.experiment ? screen.experiment.title : "")
            font.family: Theme.sans
            font.pixelSize: Theme.pageTitle
            font.weight: Theme.semibold
            font.letterSpacing: Theme.titleSpacing
            color: Theme.ink
        }

        Text {
            width: parent.width
            wrapMode: Text.Wrap
            text: "Sent over Logos Delivery and kept on each member's device, never on Logos Storage. "
                  + "Anyone with this experiment's link can read the discussion."
            font.family: Theme.sans
            font.pixelSize: Theme.meta
            color: Theme.ink3
        }

        // ---- members ----
        Flickable {
            width: parent.width
            height: Theme.touchTarget + Theme.s8
            contentWidth: memberRow.implicitWidth
            clip: true
            flickableDirection: Flickable.HorizontalFlick
            visible: screen.members.length > 0

            Row {
                id: memberRow
                spacing: Theme.s8

                Repeater {
                    model: screen.members

                    delegate: Button {
                        id: chip
                        required property var modelData
                        readonly property var shared: modelData.profile ? modelData.profile : ({})
                        height: Theme.touchTarget
                        leftPadding: Theme.s6
                        rightPadding: Theme.s12
                        Accessible.name: "Member " + (shared.alias ? shared.alias : modelData.short)
                                         + (modelData.creator ? ", the creator" : "")
                        onClicked: screen.openMember = modelData
                        contentItem: Row {
                            spacing: Theme.s8
                            Avatar {
                                anchors.verticalCenter: parent.verticalCenter
                                size: 32
                                photo: chip.shared.photo ? chip.shared.photo : ""
                                name: chip.shared.alias ? chip.shared.alias : ""
                                fallback: chip.modelData.short
                            }
                            Text {
                                anchors.verticalCenter: parent.verticalCenter
                                text: (chip.shared.alias ? chip.shared.alias : chip.modelData.short + "…")
                                      + (chip.modelData.creator ? " · creator" : "")
                                font.family: Theme.sans
                                font.pixelSize: Theme.small
                                color: Theme.ink
                            }
                        }
                        background: Rectangle {
                            radius: Theme.rPill
                            color: chip.down ? Theme.surfaceSunken : Theme.surface
                            border.width: 1
                            border.color: Theme.line
                        }
                    }
                }
            }
        }
    }

    // ---- messages ----
    ListView {
        id: list
        anchors.top: header.bottom
        anchors.topMargin: Theme.s16
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: composer.top
        anchors.leftMargin: Theme.s32
        anchors.rightMargin: Theme.s32
        clip: true
        spacing: Theme.s14
        model: screen.messages
        // Newest at the bottom, and stay there as messages arrive.
        onCountChanged: Qt.callLater(list.positionViewAtEnd)

        delegate: Row {
            id: msg
            required property var modelData
            width: list.width
            spacing: Theme.s12
            layoutDirection: modelData.mine ? Qt.RightToLeft : Qt.LeftToRight

            Avatar {
                size: 32
                photo: msg.modelData.photo ? msg.modelData.photo : ""
                name: msg.modelData.name ? msg.modelData.name : ""
                fallback: msg.modelData.short
            }

            Rectangle {
                width: Math.min(bubble.implicitWidth + Theme.s28, list.width * 0.72)
                height: bubble.implicitHeight + Theme.s20
                radius: Theme.rInner
                color: msg.modelData.mine ? Theme.accentTint : Theme.surface
                border.width: msg.modelData.mine ? 0 : 1
                border.color: Theme.line

                Column {
                    id: bubble
                    x: Theme.s14
                    y: Theme.s10
                    width: parent.width - Theme.s28
                    spacing: Theme.s4

                    Text {
                        text: screen.label(msg.modelData)
                              + (msg.modelData.creator ? " · creator" : "")
                              + " · " + screen.when(msg.modelData.at)
                        font.family: Theme.sans
                        font.pixelSize: Theme.meta
                        font.weight: Theme.medium
                        color: Theme.ink3
                    }

                    Text {
                        width: Math.min(implicitWidth, list.width * 0.72 - Theme.s28)
                        wrapMode: Text.Wrap
                        text: msg.modelData.text
                        textFormat: Text.PlainText
                        font.family: Theme.sans
                        font.pixelSize: Theme.body
                        color: Theme.ink
                        lineHeight: Theme.bodyLineHeight
                        lineHeightMode: Text.ProportionalHeight
                    }
                }
            }
        }

        Text {
            anchors.centerIn: parent
            visible: screen.messages.length === 0
            width: parent.width * 0.8
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            text: "No messages yet. Start the discussion."
            font.family: Theme.sans
            font.pixelSize: Theme.body
            color: Theme.ink3
        }
    }

    // ---- composer ----
    Column {
        id: composer
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: Theme.s24
        anchors.leftMargin: Theme.s32
        anchors.rightMargin: Theme.s32
        spacing: Theme.s8

        Text {
            visible: screen.sendError !== "" || screen.rlnBlocked || !screen.canSend
            width: parent.width
            wrapMode: Text.Wrap
            text: screen.sendError !== "" ? screen.sendError
                  : screen.rlnBlocked
                    ? "Sending needs an RLN membership on the Logos testnet. Register one with your testnet tokens, then try again."
                    : "Logos Delivery is not connected yet, so messages cannot be sent."
            font.family: Theme.sans
            font.pixelSize: Theme.small
            color: Theme.amber
        }

        Row {
            width: parent.width
            spacing: Theme.s12

            Rectangle {
                width: parent.width - sendButton.width - Theme.s12
                height: Math.max(Theme.touchTarget, Math.min(input.implicitHeight + Theme.s20, 160))
                radius: Theme.rControl
                color: Theme.surface
                border.width: 1
                border.color: input.activeFocus ? Theme.accent : Theme.inputBorder

                TextEdit {
                    id: input
                    anchors.fill: parent
                    anchors.margins: Theme.s10
                    wrapMode: TextEdit.Wrap
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    color: Theme.ink
                    selectByMouse: true
                    textFormat: TextEdit.PlainText
                    Accessible.name: "Message to the project's members"
                    Keys.onReturnPressed: function (event) {
                        // Enter sends; Shift+Enter is a new line.
                        if (event.modifiers & Qt.ShiftModifier) {
                            event.accepted = false;
                            return;
                        }
                        screen.trySend();
                    }

                    Text {
                        visible: input.text === "" && !input.activeFocus
                        text: "Write a message…"
                        font: input.font
                        color: Theme.ink3
                    }
                }
            }

            Button {
                id: sendButton
                anchors.bottom: parent.bottom
                height: Theme.touchTarget
                leftPadding: Theme.s24
                rightPadding: Theme.s24
                enabled: !screen.sending && input.text.trim() !== "" && input.text.length <= 2000
                text: screen.sending ? "Sending…" : "Send"
                Accessible.name: "Send message"
                onClicked: screen.trySend()
                contentItem: Text {
                    text: sendButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: Theme.medium
                    color: sendButton.enabled ? Theme.textOnAccent : Theme.ink3
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: !sendButton.enabled ? Theme.surfaceSunken
                         : (sendButton.down ? Theme.accentHover : Theme.accent)
                }
            }
        }

        Text {
            visible: input.text.length > 1800
            text: input.text.length + " / 2000"
            font.family: Theme.mono
            font.pixelSize: Theme.meta
            color: input.text.length > 2000 ? Theme.amber : Theme.ink3
        }
    }

    function trySend() {
        var t = input.text.trim();
        if (t === "" || t.length > 2000 || screen.sending)
            return;
        screen.send(t);
    }

    /// Called by Main once a send succeeded, so a failed send keeps its text.
    function clearComposer() {
        input.text = "";
    }

    // ---- a member's shared profile ----
    Rectangle {
        anchors.fill: parent
        visible: screen.openMember !== null
        color: Theme.backdrop

        MouseArea {
            anchors.fill: parent
            onClicked: screen.openMember = null
        }

        Rectangle {
            anchors.centerIn: parent
            width: Math.min(parent.width - Theme.s48, 420)
            height: card.implicitHeight + Theme.s48
            radius: Theme.rModal
            color: Theme.surface

            MouseArea { anchors.fill: parent }

            Column {
                id: card
                x: Theme.s24
                y: Theme.s24
                width: parent.width - Theme.s48
                spacing: Theme.s12

                readonly property var shared: screen.openMember && screen.openMember.profile
                                              ? screen.openMember.profile : ({})

                Row {
                    spacing: Theme.s14
                    Avatar {
                        size: 56
                        photo: card.shared.photo ? card.shared.photo : ""
                        name: card.shared.alias ? card.shared.alias : ""
                        fallback: screen.openMember ? screen.openMember.short : ""
                    }
                    Column {
                        anchors.verticalCenter: parent.verticalCenter
                        Text {
                            text: card.shared.alias ? card.shared.alias : "Unnamed member"
                            font.family: Theme.sans
                            font.pixelSize: Theme.cardTitle
                            font.weight: Theme.semibold
                            color: Theme.ink
                        }
                        Text {
                            text: screen.openMember ? screen.openMember.key.substring(0, 16) + "…" : ""
                            font.family: Theme.mono
                            font.pixelSize: Theme.meta
                            color: Theme.ink3
                        }
                    }
                }

                Repeater {
                    model: [
                        { key: "affiliation", label: "Affiliation" }, { key: "location", label: "Location" },
                        { key: "sex", label: "Sex" }, { key: "age", label: "Age" },
                        { key: "email", label: "Email" }, { key: "phone", label: "Phone" },
                        { key: "wallet", label: "Wallet" }
                    ]
                    delegate: Row {
                        id: line
                        required property var modelData
                        visible: card.shared[modelData.key] !== undefined
                        spacing: Theme.s12
                        Text {
                            width: 96
                            text: line.modelData.label
                            font.family: Theme.sans
                            font.pixelSize: Theme.small
                            color: Theme.ink3
                        }
                        TextEdit {
                            width: card.width - 96 - Theme.s12
                            readOnly: true
                            selectByMouse: true
                            wrapMode: TextEdit.WrapAnywhere
                            textFormat: TextEdit.PlainText
                            text: card.shared[line.modelData.key] !== undefined ? String(card.shared[line.modelData.key]) : ""
                            font.family: line.modelData.key === "wallet" ? Theme.mono : Theme.sans
                            font.pixelSize: Theme.small
                            color: Theme.ink
                        }
                    }
                }

                Text {
                    width: parent.width
                    visible: Object.keys(card.shared).length === 0
                    wrapMode: Text.Wrap
                    text: "This member has not shared anything with you."
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    color: Theme.ink3
                }

                Button {
                    id: closeCard
                    height: Theme.touchTarget
                    text: "Close"
                    Accessible.name: "Close this member's profile"
                    onClicked: screen.openMember = null
                    background: Rectangle {
                        radius: Theme.rControl
                        color: closeCard.down ? Theme.surfaceSunken : Theme.surface
                        border.width: 1
                        border.color: Theme.inputBorder
                    }
                }
            }
        }
    }
}
