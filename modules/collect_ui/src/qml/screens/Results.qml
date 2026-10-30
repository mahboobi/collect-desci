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

    /// The results.export payload, once one has been written.
    property var exportResult: null
    /// True while an export or a hide is in flight, so the buttons cannot be
    /// pressed twice.
    property bool busy: false

    signal back()
    signal exportRequested(string format, bool includeMedia)
    signal hideSubmitter(string key)
    signal showSubmitter(string key)

    readonly property var columns: screen.results && screen.results.columns ? screen.results.columns : []
    readonly property var rows: screen.results && screen.results.rows ? screen.results.rows : []
    readonly property var summary: screen.results && screen.results.summary ? screen.results.summary : ({})
    readonly property var hiddenKeys: screen.results && screen.results.hidden_keys
                                      ? screen.results.hidden_keys : []

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
                                    id: whoText
                                    objectName: "submitterCell"
                                    width: parent.width
                                    elide: Text.ElideRight
                                    // A self-chosen name if there is one, else
                                    // the key fingerprint — never the full key.
                                    // Then the wallet, shortened, when the
                                    // submitter chose to include it; the full
                                    // address is on hover and in the export.
                                    readonly property string wallet: dataRow.modelData.submitter_wallet
                                                                     ? dataRow.modelData.submitter_wallet : ""
                                    text: (dataRow.modelData.submitter_name
                                           ? dataRow.modelData.submitter_name
                                           : Core.fingerprint(dataRow.modelData.submitter))
                                          + (wallet === "" ? ""
                                             : "  \u00b7  " + (wallet.length > 14
                                                                ? wallet.slice(0, 8) + "\u2026" + wallet.slice(-4)
                                                                : wallet))
                                    font.family: dataRow.modelData.submitter_name ? Theme.sans : Theme.mono
                                    font.pixelSize: Theme.small
                                    color: Theme.ink
                                    Accessible.name: text + (wallet === "" ? "" : ". Wallet " + wallet)

                                    HoverHandler { id: whoHover }
                                    ToolTip.visible: whoHover.hovered && whoText.wallet !== ""
                                    ToolTip.text: "Wallet: " + whoText.wallet
                                }

                                Row {
                                    width: parent.width
                                    spacing: Theme.s8

                                    Text {
                                        width: parent.width - hideButton.width - Theme.s8
                                        elide: Text.ElideRight
                                        text: (dataRow.modelData.submitted_at
                                               ? dataRow.modelData.submitted_at.slice(0, 16).replace("T", " ") : "")
                                              + " \u00b7 v" + dataRow.modelData.version
                                        font.family: Theme.mono
                                        font.pixelSize: Theme.meta
                                        color: Theme.ink3
                                    }

                                    // Hiding drops the rows from the index. It
                                    // is not deletion and does not claim to
                                    // be: the documents are published and
                                    // cannot be recalled.
                                    Button {
                                        id: hideButton
                                        height: Theme.chipHeight
                                        leftPadding: Theme.s8
                                        rightPadding: Theme.s8
                                        enabled: !screen.busy
                                        text: "Hide"
                                        Accessible.name: "Hide this submitter's results"
                                        onClicked: screen.hideSubmitter(dataRow.modelData.submitter)
                                        contentItem: Text {
                                            text: hideButton.text
                                            font.family: Theme.sans
                                            font.pixelSize: Theme.meta
                                            color: Theme.ink3
                                            verticalAlignment: Text.AlignVCenter
                                        }
                                        background: Rectangle {
                                            radius: Theme.rPill
                                            color: hideButton.down ? Theme.surfaceSunken : "transparent"
                                            border.width: 1
                                            border.color: Theme.line
                                        }
                                    }
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

        // ---- hidden submitters ----
        //
        // Listed so hiding can be undone. Without this the creator can hide
        // someone and then has no way to find them again.
        Column {
            visible: screen.hiddenKeys.length > 0
            width: parent.width
            spacing: Theme.s8

            Text {
                text: "Hidden"
                font.family: Theme.sans
                font.pixelSize: Theme.sectionTitle
                font.weight: Theme.semibold
                color: Theme.ink
            }

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: "Their submissions stay on Logos Storage \u2014 hiding only removes them from this index, results and exports."
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
            }

            Repeater {
                model: screen.hiddenKeys

                delegate: Row {
                    id: hiddenRow
                    required property string modelData
                    spacing: Theme.s12

                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: Core.fingerprint(hiddenRow.modelData)
                        font.family: Theme.mono
                        font.pixelSize: Theme.small
                        color: Theme.ink2
                    }

                    Button {
                        id: showButton
                        height: Theme.chipHeight
                        leftPadding: Theme.s12
                        rightPadding: Theme.s12
                        enabled: !screen.busy
                        text: "Show again"
                        Accessible.name: "Show this submitter's results again"
                        onClicked: screen.showSubmitter(hiddenRow.modelData)
                        contentItem: Text {
                            text: showButton.text
                            font.family: Theme.sans
                            font.pixelSize: Theme.meta
                            color: Theme.ink2
                            verticalAlignment: Text.AlignVCenter
                        }
                        background: Rectangle {
                            radius: Theme.rPill
                            color: showButton.down ? Theme.surfaceSunken : "transparent"
                            border.width: 1
                            border.color: Theme.line
                        }
                    }
                }
            }
        }

        // ---- export ----
        Column {
            width: parent.width
            spacing: Theme.s12

            Text {
                text: "Export"
                font.family: Theme.sans
                font.pixelSize: Theme.sectionTitle
                font.weight: Theme.semibold
                color: Theme.ink
            }

            CheckRow {
                id: withMedia
                width: parent.width
                label: "Include the photos"
                hint: "Fetches every photo named in the table and writes them beside the file."
            }

            Row {
                spacing: Theme.s12

                Repeater {
                    model: [
                        { format: "csv", label: "Export CSV" },
                        { format: "json", label: "Export JSON" }
                    ]

                    delegate: Button {
                        id: exportButton
                        required property var modelData
                        height: Theme.touchTarget
                        leftPadding: Theme.s20
                        rightPadding: Theme.s20
                        enabled: !screen.busy && screen.rows.length > 0
                        text: exportButton.modelData.label
                        Accessible.name: exportButton.modelData.label
                        onClicked: screen.exportRequested(exportButton.modelData.format,
                                                          withMedia.on)
                        contentItem: Text {
                            text: exportButton.text
                            font.family: Theme.sans
                            font.pixelSize: Theme.small
                            font.weight: Theme.semibold
                            color: exportButton.enabled ? Theme.textOnAccent : Theme.ink3
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                        background: Rectangle {
                            radius: Theme.rControl
                            color: !exportButton.enabled
                                   ? Theme.surfaceSunken
                                   : (exportButton.down ? Theme.accentHover : Theme.accent)
                        }
                    }
                }
            }

            // Where it went. The QML side cannot open a file manager, so the
            // path is the deliverable: it is shown in full and selectable.
            Column {
                visible: screen.exportResult !== null
                width: parent.width
                spacing: Theme.s4

                Text {
                    width: parent.width
                    wrapMode: Text.Wrap
                    text: screen.exportResult
                          ? screen.exportResult.rows + " row(s) written to"
                          : ""
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    color: Theme.ink2
                }

                TextEdit {
                    width: parent.width
                    readOnly: true
                    selectByMouse: true
                    wrapMode: TextEdit.WrapAnywhere
                    text: screen.exportResult ? screen.exportResult.path : ""
                    font.family: Theme.mono
                    font.pixelSize: Theme.meta
                    color: Theme.ink
                    Accessible.name: "The path the export was written to"
                }

                Text {
                    visible: screen.exportResult && screen.exportResult.media
                             && screen.exportResult.media.requested === true
                    width: parent.width
                    wrapMode: Text.Wrap
                    text: {
                        if (!screen.exportResult || !screen.exportResult.media)
                            return "";
                        var m = screen.exportResult.media;
                        var line = m.written + " of " + m.wanted + " photo(s) fetched";
                        if (m.failed && m.failed.length > 0)
                            line += " \u2014 " + m.failed.length + " could not be fetched";
                        return line + ".";
                    }
                    font.family: Theme.sans
                    font.pixelSize: Theme.meta
                    color: screen.exportResult && screen.exportResult.media
                           && screen.exportResult.media.failed
                           && screen.exportResult.media.failed.length > 0
                           ? Theme.amber : Theme.ink3
                }
            }
        }
    }
}
