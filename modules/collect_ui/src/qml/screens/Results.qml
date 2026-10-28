// C4 — results for one form.
//
// Columns follow the protocol's fields, which is what collect_core sends. The
// creator is the only reader here; hiding is applied before the rows arrive,
// so this screen has no say in it.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect
import "../components"
import "../collect/Cells.js" as Cells

Flickable {
    id: screen

    /// The results.list payload: protocol, columns, rows, summary.
    required property var results

    signal back()

    readonly property var columns: screen.results && screen.results.columns ? screen.results.columns : []
    readonly property var rows: screen.results && screen.results.rows ? screen.results.rows : []
    readonly property var summary: screen.results && screen.results.summary ? screen.results.summary : ({})

    // Each column gets the same width; the row scrolls sideways if needed.
    readonly property int columnWidth: 160
    readonly property int metaWidth: 220

    contentWidth: width
    contentHeight: body.implicitHeight + Theme.s48 * 2
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    Column {
        id: body
        x: Theme.s32
        y: Theme.s32
        width: Math.min(screen.width - Theme.s32 * 2, 1100)
        spacing: Theme.s24

        Button {
            id: backButton
            height: Theme.chipHeight
            leftPadding: Theme.s12
            rightPadding: Theme.s14
            text: "\u2190 Back to the experiment"
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
            text: screen.results && screen.results.protocol
                  ? screen.results.protocol.name : "Results"
            font.family: Theme.sans
            font.pixelSize: Theme.pageTitle
            font.weight: Theme.semibold
            font.letterSpacing: Theme.titleSpacing
            color: Theme.ink
        }

        // ---- summary cards ----
        Flow {
            width: parent.width
            spacing: Theme.s12

            Repeater {
                model: [
                    { label: "Submissions", value: screen.summary.shown !== undefined ? screen.summary.shown : 0 },
                    { label: "Submitters", value: screen.summary.submitters !== undefined ? screen.summary.submitters : 0 },
                    { label: "In the index", value: screen.summary.indexed !== undefined ? screen.summary.indexed : 0 },
                    { label: "Hidden", value: screen.summary.hidden !== undefined ? screen.summary.hidden : 0 }
                ]

                delegate: Rectangle {
                    id: metric
                    required property var modelData
                    width: 180
                    height: 84
                    radius: Theme.rPanel
                    color: Theme.surface
                    border.width: 1
                    border.color: Theme.line

                    Column {
                        anchors.centerIn: parent
                        spacing: Theme.s4

                        Text {
                            anchors.horizontalCenter: parent.horizontalCenter
                            text: metric.modelData.value
                            font.family: Theme.mono
                            font.pixelSize: Theme.metric
                            font.weight: Theme.semibold
                            color: Theme.ink
                        }

                        Text {
                            anchors.horizontalCenter: parent.horizontalCenter
                            text: metric.modelData.label
                            font.family: Theme.sans
                            font.pixelSize: Theme.meta
                            color: Theme.ink3
                        }
                    }
                }
            }
        }

        // Received but not yet fetched is a real state on a network with no
        // listing, so it is stated rather than quietly folded into the count.
        Text {
            visible: screen.summary.not_fetched > 0
            width: parent.width
            wrapMode: Text.Wrap
            text: screen.summary.not_fetched + " submission(s) are named in the index but have not been fetched to this node yet."
            font.family: Theme.sans
            font.pixelSize: Theme.small
            color: Theme.amber
        }

        Text {
            visible: screen.rows.length === 0
            width: parent.width
            wrapMode: Text.Wrap
            text: "No submissions yet."
            font.family: Theme.sans
            font.pixelSize: Theme.body
            color: Theme.ink3
        }

        // ---- the table ----
        Flickable {
            visible: screen.rows.length > 0
            width: parent.width
            height: Math.min(table.implicitHeight, 520)
            contentWidth: table.implicitWidth
            contentHeight: table.implicitHeight
            clip: true
            flickableDirection: Flickable.HorizontalAndVerticalFlick
            boundsBehavior: Flickable.StopAtBounds

            Column {
                id: table
                spacing: 0

                // ---- header ----
                Row {
                    id: header
                    spacing: 0

                    Rectangle {
                        width: screen.metaWidth
                        height: Theme.touchTarget
                        color: Theme.surfaceSunken
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.left: parent.left
                            anchors.leftMargin: Theme.s12
                            text: "Submitter"
                            font.family: Theme.sans
                            font.pixelSize: Theme.meta
                            font.weight: Theme.semibold
                            color: Theme.ink2
                        }
                    }

                    Repeater {
                        model: screen.columns

                        delegate: Rectangle {
                            id: headCell
                            required property var modelData
                            width: screen.columnWidth
                            height: Theme.touchTarget
                            color: Theme.surfaceSunken

                            Text {
                                anchors.verticalCenter: parent.verticalCenter
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.leftMargin: Theme.s12
                                anchors.rightMargin: Theme.s8
                                elide: Text.ElideRight
                                text: headCell.modelData.label
                                font.family: Theme.sans
                                font.pixelSize: Theme.meta
                                font.weight: Theme.semibold
                                color: Theme.ink2
                            }
                        }
                    }
                }

                // ---- rows ----
                Repeater {
                    model: screen.rows

                    delegate: Row {
                        id: dataRow
                        required property var modelData
                        required property int index
                        spacing: 0

                        Rectangle {
                            width: screen.metaWidth
                            height: Theme.touchTarget + Theme.s8
                            color: dataRow.index % 2 === 0 ? Theme.surface : Theme.ground
                            border.width: 0

                            Column {
                                anchors.verticalCenter: parent.verticalCenter
                                anchors.left: parent.left
                                anchors.right: parent.right
                                anchors.leftMargin: Theme.s12
                                anchors.rightMargin: Theme.s8
                                spacing: Theme.s2

                                Text {
                                    width: parent.width
                                    elide: Text.ElideRight
                                    // A self-chosen name if there is one, else
                                    // the key fingerprint — never the full key.
                                    text: dataRow.modelData.submitter_name
                                          ? dataRow.modelData.submitter_name
                                          : Core.fingerprint(dataRow.modelData.submitter)
                                    font.family: dataRow.modelData.submitter_name ? Theme.sans : Theme.mono
                                    font.pixelSize: Theme.small
                                    color: Theme.ink
                                }

                                Text {
                                    width: parent.width
                                    elide: Text.ElideRight
                                    text: (dataRow.modelData.submitted_at
                                           ? dataRow.modelData.submitted_at.slice(0, 16).replace("T", " ") : "")
                                          + " \u00b7 v" + dataRow.modelData.version
                                    font.family: Theme.mono
                                    font.pixelSize: Theme.meta
                                    color: Theme.ink3
                                }
                            }
                        }

                        Repeater {
                            model: screen.columns

                            delegate: Rectangle {
                                id: cell
                                required property var modelData
                                width: screen.columnWidth
                                height: Theme.touchTarget + Theme.s8
                                color: dataRow.index % 2 === 0 ? Theme.surface : Theme.ground

                                Text {
                                    anchors.verticalCenter: parent.verticalCenter
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.leftMargin: Theme.s12
                                    anchors.rightMargin: Theme.s8
                                    elide: Text.ElideRight
                                    text: Cells.render(cell.modelData.type,
                                                       dataRow.modelData.values[cell.modelData.key],
                                                       cell.modelData.options)
                                    font.family: cell.modelData.type === "location"
                                                 || cell.modelData.type === "number"
                                                 || cell.modelData.type === "datetime"
                                                 ? Theme.mono : Theme.sans
                                    font.pixelSize: Theme.small
                                    color: Theme.ink
                                }
                            }
                        }
                    }
                }
            }
        }

        Text {
            width: parent.width
            wrapMode: Text.Wrap
            text: "Export and hiding a submitter are not built yet."
            font.family: Theme.sans
            font.pixelSize: Theme.meta
            color: Theme.ink3
        }
    }
}
