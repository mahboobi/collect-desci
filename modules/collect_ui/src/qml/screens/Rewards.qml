// Paying an experiment's rewards (SPEC section 11).
//
// Everything a creator needs to decide before tokens move: which account
// pays and whether it can afford it, who is paid and for what, who is not
// and why, and the total. Nothing is sent until the confirmation box is
// ticked and the button pressed; collect_core then pays one person per call,
// each checked against what is on this screen.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect
import "../components"

Flickable {
    id: screen

    required property var experiment
    // rewards.plan's data.
    property var plan: null
    property bool busy: false
    // Submitter key -> "paying" | "paid" | "failed: …" for this session's run.
    property var progress: ({})
    property string fromAddress: ""

    signal back()
    signal checkFrom(string address)
    signal payAll(string address)
    signal refresh()
    signal resolve(string payoutId, string outcome)

    readonly property var payees: screen.plan && screen.plan.payees ? screen.plan.payees : []
    readonly property var skipped: screen.plan && screen.plan.skipped ? screen.plan.skipped : []
    readonly property var payouts: screen.plan && screen.plan.payouts ? screen.plan.payouts : []
    readonly property var from: screen.plan && screen.plan.from ? screen.plan.from : null
    readonly property string symbol: screen.plan && screen.plan.reward ? screen.plan.reward.symbol : "TestToken"
    readonly property bool unresolved: screen.payouts.some(function (p) { return p.state === "sending"; })
    readonly property bool canPay: confirmBox.on && !screen.busy && !screen.unresolved
                                   && screen.payees.length > 0
                                   && !!screen.from && screen.from.covers === true
                                   && screen.from.address === screen.typedFrom

    // What is in the address box (LabelledInput's `text` only flows inward).
    property string typedFrom: screen.fromAddress

    function label(row) {
        return row.name ? row.name : Core.fingerprint(row.submitter);
    }

    function shorten(text) {
        return text && text.length > 18 ? text.slice(0, 8) + "…" + text.slice(-6) : (text ? text : "");
    }

    contentWidth: width
    contentHeight: body.implicitHeight + Theme.s48 * 2
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    Column {
        id: body
        x: Theme.s32
        y: Theme.s32
        width: Math.min(screen.width - Theme.s32 * 2, 860)
        spacing: Theme.s24

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

        Column {
            width: parent.width
            spacing: Theme.s6

            Text {
                text: screen.experiment ? screen.experiment.title : ""
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                font.weight: Theme.semibold
                color: Theme.ink3
            }

            Text {
                text: "Pay rewards"
                font.family: Theme.sans
                font.pixelSize: Theme.pageTitle
                font.weight: Theme.semibold
                font.letterSpacing: Theme.titleSpacing
                color: Theme.ink
            }

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: screen.plan && screen.plan.reward
                      ? screen.plan.reward.amount + " " + screen.symbol + " per approved submission, up to "
                        + screen.plan.reward.max_per_submitter + " per person. Paid as public LEZ testnet "
                        + "transfers from your LEZ Wallet: anyone can see them, and they cannot be undone."
                      : ""
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: Theme.ink2
            }
        }

        Text {
            objectName: "pendingWarning"
            visible: screen.plan && screen.plan.pending > 0
            width: parent.width
            wrapMode: Text.Wrap
            text: screen.plan ? (screen.plan.pending + " submission" + (screen.plan.pending === 1 ? " is" : "s are")
                                 + " still waiting for your review and will not be paid until you approve "
                                 + (screen.plan.pending === 1 ? "it" : "them") + " in Results.") : ""
            font.family: Theme.sans
            font.pixelSize: Theme.small
            color: Theme.amber
        }

        Text {
            visible: screen.unresolved
            width: parent.width
            wrapMode: Text.Wrap
            text: "A payment below has an unknown outcome: Collect stopped while sending it. Check LEZ Wallet, "
                  + "say whether it went through, and then paying can continue."
            font.family: Theme.sans
            font.pixelSize: Theme.small
            color: Theme.amber
        }

        // ---- pay from ----
        Column {
            width: parent.width
            spacing: Theme.s8

            Row {
                spacing: Theme.s12

                LabelledInput {
                    id: fromInput
                    width: Math.min(520, body.width - checkButton.width - Theme.s12)
                    label: "Pay from (your LEZ public account address)"
                    mono: true
                    text: screen.fromAddress
                    hint: screen.from && screen.from.problem ? screen.from.problem : ""
                    onEdited: function (value) { screen.typedFrom = value.trim(); }
                }

                Button {
                    id: checkButton
                    anchors.bottom: parent.bottom
                    anchors.bottomMargin: fromInput.hint !== "" ? Theme.s24 : 0
                    height: Theme.touchTarget
                    leftPadding: Theme.s16
                    rightPadding: Theme.s16
                    enabled: !screen.busy && screen.typedFrom !== ""
                    text: "Check balance"
                    Accessible.name: "Check this account's balance"
                    onClicked: screen.checkFrom(screen.typedFrom)
                    background: Rectangle {
                        radius: Theme.rControl
                        color: checkButton.down ? Theme.surfaceSunken : Theme.surface
                        border.width: 1
                        border.color: Theme.inputBorder
                    }
                }
            }

            Text {
                objectName: "balance"
                visible: !!(screen.from && screen.from.balance !== undefined)
                text: screen.from && screen.from.balance !== undefined
                      ? "Balance: " + screen.from.balance + " " + screen.symbol
                        + (screen.from.covers ? " — enough" : " — not enough for the total below")
                      : ""
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: screen.from && screen.from.covers ? Theme.accent : Theme.amber
            }

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: "Copy it from LEZ Wallet, and keep LEZ Wallet open and synced while paying."
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
            }
        }

        // ---- who is paid ----
        Column {
            width: parent.width
            spacing: Theme.s8

            Text {
                text: screen.payees.length === 0 ? "Nobody is owed a reward right now."
                      : (screen.payees.length === 1 ? "1 person to pay" : screen.payees.length + " people to pay")
                font.family: Theme.sans
                font.pixelSize: Theme.sectionTitle
                font.weight: Theme.semibold
                color: Theme.ink
            }

            Repeater {
                model: screen.payees

                delegate: Rectangle {
                    id: payeeRow
                    required property var modelData
                    readonly property string payState: screen.progress[modelData.submitter] || ""
                    width: body.width
                    height: Theme.touchTarget + Theme.s8
                    radius: Theme.rControl
                    color: Theme.surface
                    border.width: 1
                    border.color: Theme.line

                    Row {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.s12
                        anchors.rightMargin: Theme.s12
                        spacing: Theme.s12

                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 200
                            elide: Text.ElideRight
                            text: screen.label(payeeRow.modelData)
                            font.family: Theme.sans
                            font.pixelSize: Theme.small
                            font.weight: Theme.medium
                            color: Theme.ink
                        }
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 200
                            elide: Text.ElideMiddle
                            text: screen.shorten(payeeRow.modelData.wallet)
                            font.family: Theme.mono
                            font.pixelSize: Theme.meta
                            color: Theme.ink2
                        }
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 110
                            text: payeeRow.modelData.count + (payeeRow.modelData.count === 1 ? " submission" : " submissions")
                            font.family: Theme.sans
                            font.pixelSize: Theme.meta
                            color: Theme.ink3
                        }
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 140
                            text: payeeRow.modelData.amount + " " + screen.symbol
                            font.family: Theme.sans
                            font.pixelSize: Theme.small
                            font.weight: Theme.semibold
                            color: Theme.ink
                        }
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: payeeRow.payState === "paying" ? "Sending…"
                                  : (payeeRow.payState === "paid" ? "Sent ✓" : payeeRow.payState)
                            font.family: Theme.sans
                            font.pixelSize: Theme.meta
                            color: payeeRow.payState.indexOf("failed") === 0 ? Theme.amber : Theme.accent
                        }
                    }
                }
            }
        }

        // ---- who is not, and why ----
        Column {
            visible: screen.skipped.length > 0
            width: parent.width
            spacing: Theme.s6

            Text {
                text: "Not paid"
                font.family: Theme.sans
                font.pixelSize: Theme.small
                font.weight: Theme.semibold
                color: Theme.ink2
            }

            Repeater {
                model: screen.skipped
                delegate: Text {
                    required property var modelData
                    width: body.width
                    wrapMode: Text.Wrap
                    text: screen.label(modelData) + " — " + modelData.count
                          + (modelData.count === 1 ? " submission: " : " submissions: ") + modelData.reason
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    color: Theme.ink3
                }
            }
        }

        // ---- confirm ----
        Column {
            visible: screen.payees.length > 0
            width: parent.width
            spacing: Theme.s12

            Text {
                objectName: "total"
                text: "Total: " + (screen.plan ? screen.plan.total : "0") + " " + screen.symbol
                font.family: Theme.sans
                font.pixelSize: Theme.body
                font.weight: Theme.semibold
                color: Theme.ink
            }

            CheckRow {
                id: confirmBox
                objectName: "confirmPay"
                width: parent.width
                label: "Send these payments from my LEZ Wallet"
                hint: "They are public on the LEZ testnet and cannot be undone. Each person is paid once; "
                      + "Collect refuses to pay the same submission twice."
                onToggled: function (on) { confirmBox.on = on; }
            }

            Button {
                id: payButton
                objectName: "payButton"
                height: Theme.touchTarget
                leftPadding: Theme.s28
                rightPadding: Theme.s28
                enabled: screen.canPay
                text: screen.busy ? "Paying…"
                      : "Send " + screen.payees.length + (screen.payees.length === 1 ? " payment" : " payments")
                        + " (" + (screen.plan ? screen.plan.total : "0") + " " + screen.symbol + ")"
                Accessible.name: text
                onClicked: {
                    confirmBox.on = false;
                    screen.payAll(screen.typedFrom);
                }
                contentItem: Text {
                    text: payButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: Theme.medium
                    color: payButton.enabled ? Theme.textOnAccent : Theme.ink3
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: !payButton.enabled ? Theme.surfaceSunken
                         : (payButton.down ? Theme.accentHover : Theme.accent)
                }
            }

            Text {
                visible: !screen.canPay && !screen.busy
                width: parent.width
                wrapMode: Text.Wrap
                text: screen.unresolved ? "Resolve the payment with an unknown outcome first."
                      : (!screen.from || screen.from.address !== screen.typedFrom)
                        ? "Check the paying account's balance first."
                        : (screen.from.covers !== true ? "That account cannot cover the total."
                                                       : "Tick the box to confirm.")
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
            }
        }

        // ---- history ----
        Column {
            visible: screen.payouts.length > 0
            width: parent.width
            spacing: Theme.s8

            Row {
                spacing: Theme.s12

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "Payments made"
                    font.family: Theme.sans
                    font.pixelSize: Theme.sectionTitle
                    font.weight: Theme.semibold
                    color: Theme.ink
                }

                Button {
                    id: refreshButton
                    anchors.verticalCenter: parent.verticalCenter
                    height: Theme.chipHeight
                    leftPadding: Theme.s12
                    rightPadding: Theme.s12
                    enabled: !screen.busy
                    text: "Check confirmations"
                    Accessible.name: "Check whether sent payments are confirmed"
                    onClicked: screen.refresh()
                    background: Rectangle {
                        radius: Theme.rPill
                        color: refreshButton.down ? Theme.surfaceSunken : "transparent"
                        border.width: 1
                        border.color: Theme.line
                    }
                }
            }

            Repeater {
                model: screen.payouts.slice().reverse()

                delegate: Column {
                    id: paid
                    required property var modelData
                    width: body.width
                    spacing: Theme.s4

                    Text {
                        width: parent.width
                        wrapMode: Text.Wrap
                        text: screen.label(paid.modelData) + " — " + paid.modelData.amount + " " + screen.symbol
                              + " — " + ({ sending: "outcome unknown", sent: "sent, waiting for a block",
                                                confirmed: "confirmed ✓", failed: "failed" })[paid.modelData.state]
                              + (paid.modelData.tx ? " · tx " + screen.shorten(paid.modelData.tx) : "")
                              + (paid.modelData.error ? " · " + paid.modelData.error : "")
                        font.family: Theme.sans
                        font.pixelSize: Theme.small
                        color: paid.modelData.state === "failed" || paid.modelData.state === "sending"
                               ? Theme.amber : Theme.ink2
                    }

                    Row {
                        visible: paid.modelData.state === "sending"
                        spacing: Theme.s8

                        Repeater {
                            model: [ { outcome: "sent", label: "It went through" },
                                     { outcome: "failed", label: "It did not go through" } ]
                            delegate: Button {
                                id: resolveButton
                                required property var modelData
                                height: Theme.chipHeight
                                leftPadding: Theme.s12
                                rightPadding: Theme.s12
                                enabled: !screen.busy
                                text: modelData.label
                                Accessible.name: modelData.label
                                onClicked: screen.resolve(paid.modelData.id, modelData.outcome)
                                background: Rectangle {
                                    radius: Theme.rPill
                                    color: resolveButton.down ? Theme.surfaceSunken : "transparent"
                                    border.width: 1
                                    border.color: Theme.line
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
