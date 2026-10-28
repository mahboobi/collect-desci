// A date field with a calendar popup.
//
// The stored value is RFC 3339 **with an offset**, because that is what the
// contract requires and what collect_core accepts — a timestamp without one
// names no instant. A date alone cannot say that, so the picker supplies the
// time: the start of the chosen day, or its last second when `endOfDay` is
// set, in this machine's offset.
//
// MonthGrid and DayOfWeekRow come from QtQuick.Controls.Basic rather than
// plain Controls: they carry no style of their own (the delegates below are
// ours), and naming Basic explicitly means the calendar does not depend on
// whichever style the host happens to have active.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Basic as Basic
import collect

Item {
    id: picker

    required property string label
    /// True for an end date, so the day is included rather than cut off at
    /// midnight.
    property bool endOfDay: false
    /// The chosen day as YYYY-MM-DD, or "" when nothing is chosen.
    property string isoDate: ""
    property bool invalid: false

    /// The full RFC 3339 stamp, or "" when no day is chosen.
    readonly property string stamp: picker.isoDate === ""
                                    ? ""
                                    : picker.isoDate
                                      + (picker.endOfDay ? "T23:59:59" : "T00:00:00")
                                      + picker.localOffset()

    signal changed(string stamp)

    implicitWidth: 260
    implicitHeight: column.implicitHeight

    // ---- date helpers ----

    function two(n) {
        return (n < 10 ? "0" : "") + n;
    }

    /// This machine's UTC offset as +HH:MM. The only offset the UI can
    /// honestly claim without asking.
    function localOffset() {
        var minutes = -(new Date()).getTimezoneOffset();
        var sign = minutes >= 0 ? "+" : "-";
        var abs = Math.abs(minutes);
        return sign + picker.two(Math.floor(abs / 60)) + ":" + picker.two(abs % 60);
    }

    function isoOf(year, month, day) {
        return year + "-" + picker.two(month + 1) + "-" + picker.two(day);
    }

    /// The chosen day, long form, for the closed field.
    function readable() {
        if (picker.isoDate === "")
            return "";
        var parts = picker.isoDate.split("-");
        var d = new Date(Number(parts[0]), Number(parts[1]) - 1, Number(parts[2]));
        return d.toLocaleDateString(Qt.locale(), Locale.LongFormat);
    }

    Column {
        id: column
        width: parent.width
        spacing: Theme.s8

        Text {
            text: picker.label
            font.family: Theme.sans
            font.pixelSize: Theme.meta
            font.weight: Theme.medium
            color: Theme.ink2
        }

        Button {
            id: field
            width: parent.width
            height: Theme.touchTarget
            Accessible.name: picker.label + ": " + (picker.isoDate === "" ? "no date chosen" : picker.readable())
            Accessible.description: "Opens a calendar"
            onClicked: calendar.opened ? calendar.close() : calendar.open()

            contentItem: Row {
                spacing: Theme.s8

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    width: field.width - Theme.s48
                    elide: Text.ElideRight
                    text: picker.isoDate === "" ? "Choose a date" : picker.readable()
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    color: picker.isoDate === "" ? Theme.ink3 : Theme.ink
                }

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: "\u25be"
                    font.pixelSize: Theme.body
                    color: Theme.ink3
                }
            }
            background: Rectangle {
                radius: Theme.rControl
                color: Theme.surface
                border.width: 1
                border.color: calendar.opened ? Theme.accent
                            : (picker.invalid ? Theme.amber : Theme.inputBorder)
            }
        }

        // The exact stored value. Shown rather than hidden: everything here
        // becomes public, and the offset is part of what gets stored.
        Text {
            visible: picker.stamp !== ""
            text: picker.stamp
            font.family: Theme.mono
            font.pixelSize: Theme.meta
            color: Theme.ink3
        }
    }

    Popup {
        id: calendar

        y: field.y + field.height + Theme.s6
        width: 300
        padding: Theme.s12
        modal: false
        focus: true
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutsideParent

        // Which month is on screen. Starts on the chosen day, else today.
        property int shownYear: 0
        property int shownMonth: 0

        onAboutToShow: {
            var base = picker.isoDate === "" ? new Date() : new Date(picker.isoDate + "T00:00:00");
            calendar.shownYear = base.getFullYear();
            calendar.shownMonth = base.getMonth();
        }

        background: Rectangle {
            radius: Theme.rPanel
            color: Theme.surface
            border.width: 1
            border.color: Theme.line
        }

        contentItem: Column {
            spacing: Theme.s10

            // ---- month header ----
            Row {
                width: parent.width

                Button {
                    id: previousMonth
                    width: Theme.touchTarget
                    height: Theme.touchTarget
                    text: "\u2039"
                    Accessible.name: "Previous month"
                    onClicked: {
                        if (calendar.shownMonth === 0) {
                            calendar.shownMonth = 11;
                            calendar.shownYear -= 1;
                        } else {
                            calendar.shownMonth -= 1;
                        }
                    }
                    contentItem: Text {
                        text: previousMonth.text
                        font.pixelSize: Theme.cardTitle
                        color: Theme.ink2
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }
                    background: Rectangle {
                        radius: Theme.rControl
                        color: previousMonth.down ? Theme.surfaceSunken : "transparent"
                    }
                }

                Text {
                    width: parent.width - Theme.touchTarget * 2
                    height: Theme.touchTarget
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    text: Qt.locale().standaloneMonthName(calendar.shownMonth) + " " + calendar.shownYear
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: Theme.semibold
                    color: Theme.ink
                }

                Button {
                    id: nextMonth
                    width: Theme.touchTarget
                    height: Theme.touchTarget
                    text: "\u203a"
                    Accessible.name: "Next month"
                    onClicked: {
                        if (calendar.shownMonth === 11) {
                            calendar.shownMonth = 0;
                            calendar.shownYear += 1;
                        } else {
                            calendar.shownMonth += 1;
                        }
                    }
                    contentItem: Text {
                        text: nextMonth.text
                        font.pixelSize: Theme.cardTitle
                        color: Theme.ink2
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                    }
                    background: Rectangle {
                        radius: Theme.rControl
                        color: nextMonth.down ? Theme.surfaceSunken : "transparent"
                    }
                }
            }

            Basic.DayOfWeekRow {
                width: parent.width
                locale: Qt.locale()
                delegate: Text {
                    required property var model
                    horizontalAlignment: Text.AlignHCenter
                    text: model.shortName
                    font.family: Theme.sans
                    font.pixelSize: Theme.overline
                    font.weight: Theme.semibold
                    color: Theme.ink3
                }
            }

            Basic.MonthGrid {
                id: grid
                width: parent.width
                month: calendar.shownMonth
                year: calendar.shownYear
                locale: Qt.locale()

                delegate: Rectangle {
                    id: day
                    required property var model
                    readonly property string iso: picker.isoOf(day.model.year, day.model.month, day.model.day)
                    readonly property bool chosen: picker.isoDate === day.iso
                    // Days spilling in from the neighbouring months.
                    readonly property bool outside: day.model.month !== calendar.shownMonth

                    implicitWidth: Theme.touchTarget
                    implicitHeight: Theme.touchTarget - Theme.s6
                    radius: Theme.rControl
                    color: day.chosen ? Theme.accent
                         : (dayHover.hovered ? Theme.accentTintSoft : "transparent")

                    HoverHandler { id: dayHover }

                    Text {
                        anchors.centerIn: parent
                        text: day.model.day
                        font.family: Theme.sans
                        font.pixelSize: Theme.small
                        font.weight: day.chosen ? Theme.semibold : Theme.regular
                        color: day.chosen ? Theme.textOnAccent
                             : (day.outside ? Theme.dashed : Theme.ink)
                    }

                    TapHandler {
                        onTapped: {
                            picker.isoDate = day.iso;
                            picker.changed(picker.stamp);
                            calendar.close();
                        }
                    }
                }
            }

            // Escape hatch for a date far from today.
            Button {
                id: todayButton
                width: parent.width
                height: Theme.chipHeight
                text: "Today"
                Accessible.name: "Choose today"
                onClicked: {
                    var now = new Date();
                    picker.isoDate = picker.isoOf(now.getFullYear(), now.getMonth(), now.getDate());
                    picker.changed(picker.stamp);
                    calendar.close();
                }
                contentItem: Text {
                    text: todayButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    color: Theme.accent
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rPill
                    color: todayButton.down ? Theme.accentTint : "transparent"
                }
            }
        }
    }
}
