// S1 — the experiment as a submitter sees it after opening a link.
//
// Everything a reader needs to decide whether to trust and take part: who made
// it, the key fingerprint, the dates, the public-data warning, and the forms,
// which are listed straight away.
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
    /// The creator filling in their own form, rather than editing it.
    signal fillInProtocol(string protocolId)
    signal openResults(string protocolId)
    signal addProtocol()
    signal editExperiment()
    signal openDiscussion()
    signal back()

    // A creator looking at their own experiment is editing, not contributing.
    property bool mine: false

    /// Whether the manifest is on Logos Storage, and where. A creator needs
    /// to see this: it is the difference between an experiment that exists
    /// and one that only exists on this machine, and until now the screen
    /// did not say.
    property bool published: false
    property string cid: ""
    property var publishError: null
    signal publishNow()

    /// The cover image as a data URI from collect_core, or "" for none.
    property string thumbnail: ""
    /// A thumbnail upload in flight, and the last one's failure if any.
    property bool thumbnailBusy: false
    property string thumbnailError: ""
    signal pickThumbnail(string url)
    signal clearThumbnail()
    /// Ask the backend's native chooser for a cover (see Profile.nativePicker).
    signal requestThumbnailPick()
    property bool nativePicker: false

    // ---- rewards ----
    readonly property var reward: Core.rewardOf(screen.experiment)
    readonly property bool ended: !!(screen.experiment && screen.experiment.ends_at)
                                  && screen.nowMs >= Date.parse(screen.experiment.ends_at)
    property bool rewardBusy: false
    property var rewardProblems: ({})
    signal setReward(string amount, int maxPerSubmitter)
    signal openRewards()

    /// This node's submissions to this experiment (submissions.mine), each
    /// waiting for the creator's receipt or received.
    property var mySubmissions: []
    // What has been typed. LabelledInput's `text` only flows inward, so the
    // typed value is followed through `edited`; null means untouched.
    property var typedAmount: null
    property var typedMax: null
    property string typedCoverPath: ""

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

        // The cover. A submitter only sees it when there is one; the creator
        // always does, with the controls to set it.
        Column {
            id: cover
            visible: screen.thumbnail !== "" || screen.mine
            width: parent.width
            spacing: Theme.s8

            Thumbnail {
                objectName: "coverImage"
                width: parent.width
                source: screen.thumbnail
                title: screen.experiment ? screen.experiment.title : ""
                emptyNote: screen.mine ? "No cover image yet" : ""
                opacity: screen.thumbnailBusy ? 0.5 : 1
            }

            Row {
                visible: screen.mine
                spacing: Theme.s8

                Button {
                    id: chooseCover
                    objectName: "chooseCover"
                    height: Theme.chipHeight
                    leftPadding: Theme.s14
                    rightPadding: Theme.s14
                    enabled: !screen.thumbnailBusy
                    text: screen.thumbnailBusy ? "Uploading\u2026"
                          : (screen.thumbnail !== "" ? "Change cover image" : "Add cover image")
                    Accessible.name: text
                    onClicked: {
                        if (screen.nativePicker)
                            screen.requestThumbnailPick();
                        else if (coverPicker.item)
                            coverPicker.item.open();
                        else
                            coverPath.visible = true;
                    }
                    contentItem: Text {
                        text: chooseCover.text
                        font.family: Theme.sans
                        font.pixelSize: Theme.small
                        font.weight: Theme.medium
                        color: Theme.accent
                        verticalAlignment: Text.AlignVCenter
                    }
                    background: Rectangle {
                        radius: Theme.rPill
                        color: chooseCover.down ? Theme.surfaceSunken : Theme.surface
                        border.width: 1
                        border.color: chooseCover.activeFocus ? Theme.accent : Theme.line
                    }
                }

                Button {
                    id: removeCover
                    objectName: "removeCover"
                    visible: screen.thumbnail !== ""
                    height: Theme.chipHeight
                    leftPadding: Theme.s14
                    rightPadding: Theme.s14
                    enabled: !screen.thumbnailBusy
                    text: "Remove"
                    Accessible.name: "Remove the cover image"
                    onClicked: screen.clearThumbnail()
                    contentItem: Text {
                        text: removeCover.text
                        font.family: Theme.sans
                        font.pixelSize: Theme.small
                        color: Theme.ink2
                        verticalAlignment: Text.AlignVCenter
                    }
                    background: Rectangle {
                        radius: Theme.rPill
                        color: removeCover.down ? Theme.surfaceSunken : "transparent"
                        border.width: 1
                        border.color: removeCover.activeFocus ? Theme.accent : Theme.line
                    }
                }
            }

            Text {
                visible: screen.mine
                width: parent.width
                wrapMode: Text.Wrap
                text: "Shown on the card and here to everyone with the link. It is cropped to 16:9, "
                      + "location and camera details are removed, and it is stored publicly on Logos Storage."
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
            }

            // Shown when the sandbox has no file dialog.
            Row {
                id: coverPath
                visible: false
                spacing: Theme.s8

                LabelledInput {
                    id: coverPathInput
                    width: Math.min(420, cover.width - coverUse.width - Theme.s8)
                    label: "Path to an image file"
                    mono: true
                    hint: "This app cannot open a file picker here, so paste the file's full path."
                    onEdited: function (value) { screen.typedCoverPath = value; }
                }

                Button {
                    id: coverUse
                    anchors.bottom: parent.bottom
                    anchors.bottomMargin: coverPathInput.hint !== "" ? Theme.s24 : 0
                    height: Theme.touchTarget
                    text: "Use"
                    Accessible.name: "Use this image as the cover"
                    onClicked: screen.pickThumbnail(screen.typedCoverPath)
                    background: Rectangle {
                        radius: Theme.rControl
                        color: Theme.surface
                        border.width: 1
                        border.color: Theme.inputBorder
                    }
                }
            }

            Text {
                visible: screen.thumbnailError !== ""
                width: parent.width
                wrapMode: Text.Wrap
                text: screen.thumbnailError
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: Theme.amber
            }
        }

        Row {
            width: parent.width
            spacing: Theme.s12

            Text {
                width: parent.width - discussButton.width - Theme.s12
                       - (editDetails.visible ? editDetails.width + Theme.s12 : 0)
                wrapMode: Text.Wrap
                text: screen.experiment ? screen.experiment.title : ""
                font.family: Theme.sans
                font.pixelSize: Theme.pageTitle
                font.weight: Theme.semibold
                font.letterSpacing: Theme.titleSpacing
                color: Theme.ink
            }

            Button {
                id: discussButton
                anchors.top: parent.top
                height: Theme.chipHeight
                leftPadding: Theme.s14
                rightPadding: Theme.s14
                text: "Discussion"
                Accessible.name: "Open this project's discussion"
                onClicked: screen.openDiscussion()
                contentItem: Text {
                    text: discussButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    font.weight: Theme.medium
                    color: Theme.accent
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rPill
                    color: discussButton.down ? Theme.surfaceSunken : "transparent"
                    border.width: 1
                    border.color: Theme.line
                }
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

        // ---- where it is ----
        Row {
            width: parent.width
            spacing: Theme.s12

            Column {
                width: parent.width - (retryPublish.visible ? retryPublish.width + Theme.s12 : 0)
                spacing: Theme.s2

                Text {
                    width: parent.width
                    elide: Text.ElideMiddle
                    text: screen.published
                          ? "On Logos Storage \u00b7 " + screen.cid
                          : (screen.publishError
                             ? "Not published \u00b7 " + screen.publishError.message
                             : "Not published yet \u2014 saved on this device only")
                    font.family: screen.published ? Theme.mono : Theme.sans
                    font.pixelSize: Theme.meta
                    color: screen.published ? Theme.ink2 : Theme.amber
                    Accessible.name: screen.published ? "Published at " + screen.cid : "Not published"
                }
            }

            Button {
                id: retryPublish
                visible: screen.mine && !screen.published
                anchors.verticalCenter: parent.verticalCenter
                height: Theme.chipHeight
                leftPadding: Theme.s14
                rightPadding: Theme.s14
                text: "Publish now"
                Accessible.name: "Publish this experiment to Logos Storage"
                onClicked: screen.publishNow()
                contentItem: Text {
                    text: retryPublish.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    font.weight: Theme.medium
                    color: Theme.accent
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rPill
                    color: retryPublish.down ? Theme.surfaceSunken : "transparent"
                    border.width: 1
                    border.color: Theme.accent
                }
            }
        }

        // ---- reward ----
        // Everyone sees the offer; the creator sets it, and pays it here
        // once the experiment has ended.
        Rectangle {
            id: rewardCard
            objectName: "rewardCard"
            visible: screen.reward !== null || screen.mine
            width: parent.width
            implicitHeight: rewardBody.implicitHeight + Theme.s32
            radius: Theme.rPanel
            color: Theme.surface
            border.width: 1
            border.color: Theme.line

            Column {
                id: rewardBody
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                anchors.margins: Theme.s16
                spacing: Theme.s10

                Text {
                    text: "Reward"
                    font.family: Theme.sans
                    font.pixelSize: Theme.overline
                    font.weight: Theme.semibold
                    font.letterSpacing: Theme.overlineSpacing
                    font.capitalization: Font.AllUppercase
                    color: Theme.ink3
                }

                Text {
                    objectName: "rewardTerms"
                    width: parent.width
                    wrapMode: Text.Wrap
                    text: screen.reward
                          ? screen.reward.amount + " " + screen.reward.symbol + " (LEZ testnet) per approved submission, "
                            + "up to " + screen.reward.max + " per person, paid after the experiment ends."
                            + (screen.mine ? "" : " To be paid, include your wallet address when you submit.")
                          : "No reward. You can offer LEZ TestToken for each submission you approve."
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    color: Theme.ink
                }

                // The creator's controls. Locked once anything is paid
                // (collect_core refuses, and says so).
                Row {
                    visible: screen.mine && !screen.ended
                    spacing: Theme.s12

                    LabelledInput {
                        id: rewardAmount
                        width: 200
                        label: "TestToken per submission"
                        text: screen.reward ? screen.reward.amount : ""
                        hint: screen.rewardProblems.amount ? screen.rewardProblems.amount : ""
                        inputHints: Qt.ImhFormattedNumbersOnly
                        onEdited: function (value) { screen.typedAmount = value; }
                    }

                    LabelledInput {
                        id: rewardMax
                        width: 160
                        label: "Most per person"
                        text: screen.reward ? String(screen.reward.max) : "1"
                        hint: screen.rewardProblems.maxPerSubmitter ? screen.rewardProblems.maxPerSubmitter : ""
                        inputHints: Qt.ImhDigitsOnly
                        onEdited: function (value) { screen.typedMax = value; }
                    }

                    Button {
                        id: saveReward
                        objectName: "saveReward"
                        anchors.bottom: parent.bottom
                        anchors.bottomMargin: (rewardAmount.hint !== "" || rewardMax.hint !== "") ? Theme.s24 : 0
                        height: Theme.touchTarget
                        leftPadding: Theme.s16
                        rightPadding: Theme.s16
                        enabled: !screen.rewardBusy
                        text: screen.reward ? "Update reward" : "Offer reward"
                        Accessible.name: text
                        onClicked: screen.setReward(
                                       screen.typedAmount !== null ? screen.typedAmount : rewardAmount.text,
                                       parseInt(screen.typedMax !== null ? screen.typedMax : rewardMax.text, 10) || 0)
                        contentItem: Text {
                            text: saveReward.text
                            font.family: Theme.sans
                            font.pixelSize: Theme.small
                            font.weight: Theme.medium
                            color: Theme.accent
                            verticalAlignment: Text.AlignVCenter
                        }
                        background: Rectangle {
                            radius: Theme.rControl
                            color: saveReward.down ? Theme.accentTint : Theme.surface
                            border.width: 1
                            border.color: Theme.accent
                        }
                    }

                    Button {
                        id: removeReward
                        visible: screen.reward !== null
                        anchors.bottom: saveReward.bottom
                        height: Theme.touchTarget
                        leftPadding: Theme.s14
                        rightPadding: Theme.s14
                        enabled: !screen.rewardBusy
                        text: "Remove"
                        Accessible.name: "Remove the reward"
                        onClicked: screen.setReward("", 1)
                        contentItem: Text {
                            text: removeReward.text
                            font.family: Theme.sans
                            font.pixelSize: Theme.small
                            color: Theme.ink2
                            verticalAlignment: Text.AlignVCenter
                        }
                        background: Rectangle {
                            radius: Theme.rControl
                            color: removeReward.down ? Theme.surfaceSunken : "transparent"
                            border.width: 1
                            border.color: Theme.line
                        }
                    }
                }

                Text {
                    visible: screen.mine && screen.reward !== null && !screen.ended
                    width: parent.width
                    wrapMode: Text.Wrap
                    text: "Approve submissions in each form's Results. Paying opens here once the experiment ends, "
                          + "and nothing is sent until you confirm it."
                    font.family: Theme.sans
                    font.pixelSize: Theme.meta
                    color: Theme.ink3
                }

                Button {
                    id: payRewards
                    objectName: "payRewards"
                    visible: screen.mine && screen.reward !== null && screen.ended
                    height: Theme.touchTarget
                    leftPadding: Theme.s24
                    rightPadding: Theme.s24
                    text: "Pay rewards\u2026"
                    Accessible.name: "Review and pay this experiment's rewards"
                    onClicked: screen.openRewards()
                    contentItem: Text {
                        text: payRewards.text
                        font.family: Theme.sans
                        font.pixelSize: Theme.body
                        font.weight: Theme.medium
                        color: Theme.textOnAccent
                        verticalAlignment: Text.AlignVCenter
                    }
                    background: Rectangle {
                        radius: Theme.rControl
                        color: payRewards.down ? Theme.accentHover : Theme.accent
                    }
                }
            }
        }

        // ---- your submissions ----
        // Only on someone else's experiment: the creator indexes their own
        // directly and needs no receipt.
        Column {
            objectName: "mySubmissions"
            visible: !screen.mine && screen.mySubmissions.length > 0
            width: parent.width
            spacing: Theme.s8

            Text {
                text: "Your submissions"
                font.family: Theme.sans
                font.pixelSize: Theme.sectionTitle
                font.weight: Theme.semibold
                color: Theme.ink
            }

            Repeater {
                model: screen.mySubmissions

                delegate: Rectangle {
                    id: sent
                    required property var modelData
                    readonly property bool received: modelData.state === "received"
                    readonly property var form: Core.protocolById(modelData.protocol)
                    width: body.width
                    implicitHeight: sentText.implicitHeight + Theme.s24
                    radius: Theme.rControl
                    color: Theme.surface
                    border.width: 1
                    border.color: Theme.line

                    Text {
                        id: sentText
                        anchors.left: parent.left
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        anchors.margins: Theme.s12
                        wrapMode: Text.Wrap
                        textFormat: Text.PlainText
                        text: (sent.form ? sent.form.name : sent.modelData.protocol) + " \u00b7 "
                              + String(sent.modelData.submitted_at).slice(0, 16).replace("T", " ") + " \u2014 "
                              + (sent.received
                                 ? "Received by the creator \u2713"
                                 : "Waiting for the creator\u2026 announced " + sent.modelData.attempts
                                   + (sent.modelData.attempts === 1 ? " time" : " times")
                                   + (sent.modelData.last_error ? ". Last try: " + sent.modelData.last_error : ""))
                        font.family: Theme.sans
                        font.pixelSize: Theme.small
                        color: sent.received ? Theme.accent
                             : (sent.modelData.last_error ? Theme.amber : Theme.ink2)
                    }
                }
            }

            Text {
                visible: screen.mySubmissions.some(function (m) { return m.state !== "received"; })
                width: parent.width
                wrapMode: Text.Wrap
                text: "Your answers are signed and saved on Logos Storage already. Collect keeps announcing them, "
                      + "less often over time, until the creator's Basecamp confirms it has them."
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
            }
        }

        PublicNotice {}

        // ---- the forms ----
        // Shown straight away, below the public-data notice. There used to be
        // a "contribute" step in front of them; it only added a click.
        Column {
            objectName: "formsSection"
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

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: Core.protocolsOf(screen.experiment).length === 0
                      ? "This experiment has no forms yet."
                      : (!screen.open ? "This experiment is not accepting submissions right now." : "")
                visible: text !== ""
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: screen.open ? Theme.ink3 : Theme.amber
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
                                   - (screen.mine ? resultsButton.width + Theme.s12 : 0)
                                   - (submitAsCreator.visible ? submitAsCreator.width + Theme.s12 : 0)
                                   - Theme.s24
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
                            // The creator's own "Fill in". A creator
                            // submitting to their own experiment is an
                            // ordinary submitter — same form, same
                            // validation, same signature — and without this
                            // there is no way to try a form you just built.
                            id: submitAsCreator
                            visible: screen.mine && card.fillable
                            anchors.verticalCenter: parent.verticalCenter
                            height: Theme.touchTarget
                            leftPadding: Theme.s16
                            rightPadding: Theme.s16
                            text: "Fill in"
                            Accessible.name: (card.doc ? card.doc.name : card.modelData.id)
                                             + ": fill this form in yourself"
                            onClicked: screen.fillInProtocol(card.modelData.id)
                            contentItem: Text {
                                text: submitAsCreator.text
                                font.family: Theme.sans
                                font.pixelSize: Theme.small
                                font.weight: Theme.medium
                                color: Theme.accent
                                horizontalAlignment: Text.AlignHCenter
                                verticalAlignment: Text.AlignVCenter
                            }
                            background: Rectangle {
                                radius: Theme.rControl
                                color: submitAsCreator.down ? Theme.surfaceSunken : "transparent"
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

    Loader {
        id: coverPicker
        active: screen.mine && !screen.nativePicker
        source: "../components/PhotoPicker.qml"
        onLoaded: item.title = "Choose a cover image"
        onStatusChanged: {
            if (status === Loader.Error)
                console.warn("collect: no file dialog in this sandbox; offering a typed path");
        }
    }

    Connections {
        target: coverPicker.item
        ignoreUnknownSignals: true
        function onPicked(url) { screen.pickThumbnail(url); }
    }
}
