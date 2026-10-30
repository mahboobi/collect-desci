// One experiment on the dashboard.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect

Rectangle {
    id: card

    required property var model
    // Creator cards offer editing; submitter cards offer contributing.
    readonly property bool mine: model.mine === true
    readonly property string standing: model.standing ? model.standing : "draft"

    signal opened()

    implicitHeight: content.implicitHeight + Theme.s32
    radius: Theme.rIdeaCard
    color: Theme.surface
    border.width: 1
    border.color: hover.hovered ? Theme.accent : Theme.line

    HoverHandler { id: hover }

    // The whole card is the target, so there is one obvious thing to hit.
    TapHandler { onTapped: card.opened() }

    Column {
        id: content
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: Theme.s16
        spacing: Theme.s10

        Thumbnail {
            objectName: "cardThumbnail"
            width: content.width
            source: card.model.thumbnail ? card.model.thumbnail : ""
            title: card.model.title ? card.model.title : ""
        }

        Row {
            width: parent.width
            spacing: Theme.s8

            Badge {
                text: card.standing === "open" ? "Open"
                    : (card.standing === "ended" ? "Ended" : "Draft")
                variant: card.standing === "open" ? "done"
                       : (card.standing === "ended" ? "neutral" : "current")
            }

            Badge {
                visible: card.mine
                text: "Yours"
                variant: "current"
            }
        }

        Text {
            width: content.width
            wrapMode: Text.Wrap
            maximumLineCount: 2
            elide: Text.ElideRight
            text: card.model.title ? card.model.title : "Untitled"
            font.family: Theme.sans
            font.pixelSize: Theme.cardTitle
            font.weight: Theme.semibold
            color: Theme.ink
        }

        Text {
            visible: !!card.model.description
            width: content.width
            wrapMode: Text.Wrap
            maximumLineCount: 2
            elide: Text.ElideRight
            text: card.model.description ? card.model.description : ""
            font.family: Theme.sans
            font.pixelSize: Theme.small
            color: Theme.ink2
        }

        Text {
            text: {
                var forms = card.model.protocol_count === 1 ? "1 form" : card.model.protocol_count + " forms";
                var dates = card.model.starts_at
                            ? card.model.starts_at.slice(0, 10) + " to " + card.model.ends_at.slice(0, 10)
                            : "";
                return forms + (dates ? " \u00b7 " + dates : "");
            }
            font.family: Theme.sans
            font.pixelSize: Theme.meta
            color: Theme.ink3
        }

        Button {
            id: action
            height: Theme.touchTarget
            leftPadding: Theme.s20
            rightPadding: Theme.s20
            // The two roles want different verbs on the same card.
            text: card.mine ? "Open and edit" : "View details"
            Accessible.name: text + ": " + (card.model.title ? card.model.title : "untitled experiment")
            onClicked: card.opened()

            contentItem: Text {
                text: action.text
                font.family: Theme.sans
                font.pixelSize: Theme.small
                font.weight: Theme.medium
                color: Theme.accent
                verticalAlignment: Text.AlignVCenter
            }
            background: Rectangle {
                radius: Theme.rControl
                color: action.down ? Theme.accentTint : "transparent"
                border.width: 1
                border.color: Theme.accent
            }
        }
    }
}
