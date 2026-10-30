// S2 — the generated form.
//
// Instructions at the top, then the fields in protocol order, then Submit.
// Submit stays disabled until every required field is answered, which is what
// the artboard shows and what SPEC section 3.2 step 2 asks for.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect
import "../components"

Flickable {
    id: screen

    required property var experiment
    required property var protocol

    // Whether the experiment is accepting submissions right now. S1 gates
    // entry, but a form left open past the end date must stop too.
    property bool accepting: true

    signal submitted(string cid)
    signal cancelled()

    /// The wallet address saved in this person's profile, or "" for none.
    /// Only shown, never sent: collect_core reads the address from the
    /// profile itself when `includeWallet` is set.
    property string wallet: ""
    /// Off for every new form. A wallet in a submission is public forever,
    /// so it is a choice made each time, not a setting.
    property bool includeWallet: false

    readonly property alias complete: renderer.complete

    contentWidth: width
    contentHeight: body.implicitHeight + Theme.s48 * 2
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    Column {
        id: body
        x: Theme.s32
        y: Theme.s32
        width: Math.min(screen.width - Theme.s32 * 2, 760)
        spacing: Theme.s28

        // ---- heading ----
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
                width: parent.width
                wrapMode: Text.Wrap
                text: screen.protocol ? screen.protocol.name : ""
                font.family: Theme.sans
                font.pixelSize: Theme.pageTitle
                font.weight: Theme.semibold
                font.letterSpacing: Theme.titleSpacing
                color: Theme.ink
            }
        }

        // ---- instructions ----
        Rectangle {
            visible: !!(screen.protocol && screen.protocol.instructions)
            width: parent.width
            implicitHeight: instructions.implicitHeight + Theme.s32
            radius: Theme.rPanel
            color: Theme.surfaceSunken

            Column {
                id: instructions
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                anchors.leftMargin: Theme.s20
                anchors.rightMargin: Theme.s20
                spacing: Theme.s8

                Text {
                    text: "Instructions from the creator"
                    font.family: Theme.sans
                    font.pixelSize: Theme.overline
                    font.weight: Theme.semibold
                    font.letterSpacing: Theme.overlineSpacing
                    font.capitalization: Font.AllUppercase
                    color: Theme.ink3
                }

                Text {
                    width: instructions.width
                    wrapMode: Text.Wrap
                    text: screen.protocol ? screen.protocol.instructions : ""
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    color: Theme.ink
                    lineHeight: Theme.bodyLineHeight
                    lineHeightMode: Text.ProportionalHeight
                }

                Text {
                    text: "Fields marked \"required\" must be filled in. Form version "
                          + (screen.protocol ? screen.protocol.version : "")
                          + "."
                    font.family: Theme.sans
                    font.pixelSize: Theme.meta
                    color: Theme.ink3
                }
            }
        }

        // ---- closed notice ----
        Rectangle {
            visible: !screen.accepting
            width: parent.width
            implicitHeight: Theme.touchTarget + Theme.s8
            radius: Theme.rPanel
            color: Theme.slateTint

            Text {
                anchors.centerIn: parent
                text: "This form is not accepting submissions."
                font.family: Theme.sans
                font.pixelSize: Theme.body
                color: Theme.slate
            }
        }

        // ---- the fields ----
        FormRenderer {
            id: renderer
            width: parent.width
            enabled: screen.accepting
            protocol: screen.protocol
        }

        // ---- what happens on submit ----
        Column {
            width: parent.width
            spacing: Theme.s10

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: renderer.complete
                      ? "Ready to submit."
                      : [renderer.problemLabels.length > 0
                             ? "To fix: " + renderer.problemLabels.join("; ") : "",
                         renderer.missingLabels.length > 0
                             ? "Still needed: " + renderer.missingLabels.join(", ") : ""]
                        .filter(function (s) { return s !== ""; }).join(". ")
                        || "Fill in the form to submit."
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: renderer.complete ? Theme.accent : Theme.ink2
            }

            CheckRow {
                id: walletRow
                objectName: "includeWallet"
                visible: screen.wallet !== ""
                width: parent.width
                label: "Include my wallet address with this submission"
                hint: (Core.rewardOf(screen.experiment)
                       ? "This experiment pays " + Core.rewardOf(screen.experiment).amount
                         + " TestToken per approved submission, to this address. " : "")
                      + screen.wallet + " will be saved with your answers, publicly and permanently. "
                      + "Anyone can see it, and it cannot be removed later."
                on: screen.includeWallet
                onToggled: function (on) { screen.includeWallet = on; }
            }

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: "Signed with your Basecamp key and saved publicly on Logos Storage."
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
            }
        }

        // ---- actions ----
        Row {
            spacing: Theme.s12

            Button {
                id: submitButton
                height: Theme.touchTarget
                leftPadding: Theme.s28
                rightPadding: Theme.s28
                // Disabled rather than hidden, so the reason above it has
                // something to explain.
                enabled: screen.accepting && renderer.complete
                text: "Submit"
                Accessible.name: "Submit this form"
                onClicked: screen.doSubmit()

                contentItem: Text {
                    text: submitButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: Theme.medium
                    color: submitButton.enabled ? Theme.textOnAccent : Theme.ink3
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: !submitButton.enabled ? Theme.surfaceSunken
                         : (submitButton.down ? Theme.accentHover : Theme.accent)
                }
            }

            Button {
                id: draftButton
                visible: !!(screen.protocol && screen.protocol.allow_drafts)
                height: Theme.touchTarget
                leftPadding: Theme.s20
                rightPadding: Theme.s20
                text: "Save draft"
                Accessible.name: "Save this form as a draft"

                contentItem: Text {
                    text: draftButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    color: Theme.ink
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: draftButton.down ? Theme.surfaceSunken : Theme.surface
                    border.width: 1
                    border.color: Theme.inputBorder
                }
            }
        }
    }

    function doSubmit() {
        renderer.showProblems = true;
        if (!renderer.complete) {
            renderer.focusFirstMissing();
            return;
        }
        var draft = {
            experiment: screen.experiment ? screen.experiment.id : "",
            protocol: {
                id: screen.protocol.id,
                version: screen.protocol.version
            },
            values: renderer.values(),
            includeWallet: screen.includeWallet && screen.wallet !== ""
        };
        Core.submit(draft, function (result) {
            if (result.ok) {
                screen.submitted(result.data.cid);
                return;
            }
            // A refused submission has to say so. Returning quietly here is
            // how "I pressed Submit and nothing happened" gets built.
            Core.reportFailure(result.error ? result.error.message : "The submission was refused.");
        });
    }
}
