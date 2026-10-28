// The DatePicker's stored value.
//
// The date is the easy part; the stamp is what the contract cares about, so
// that is what these check.
import QtQuick
import QtTest
import collect
import "../src/qml/components"

Item {
    id: host
    width: 400
    height: 400

    Component {
        id: pickerComponent
        DatePicker {}
    }

    function make(endOfDay) {
        return pickerComponent.createObject(host, { label: "When", endOfDay: endOfDay });
    }

    TestCase {
        name: "DatePicker"
        when: windowShown

        function test_no_date_means_no_stamp() {
            var p = make(false);
            compare(p.isoDate, "");
            compare(p.stamp, "");
            p.destroy();
        }

        function test_start_date_is_the_first_instant_of_the_day() {
            var p = make(false);
            p.isoDate = "2026-11-01";
            verify(p.stamp.indexOf("2026-11-01T00:00:00") === 0, p.stamp);
            p.destroy();
        }

        function test_end_date_includes_the_whole_day() {
            // Otherwise choosing today as the end would close the experiment
            // before it opened.
            var p = make(true);
            p.isoDate = "2026-11-01";
            verify(p.stamp.indexOf("2026-11-01T23:59:59") === 0, p.stamp);
            p.destroy();
        }

        function test_stamp_always_carries_an_offset() {
            // collect_core rejects a timestamp without one, because without
            // an offset it names no instant.
            var p = make(false);
            p.isoDate = "2026-11-01";
            verify(/[+-]\d{2}:\d{2}$/.test(p.stamp), p.stamp);
            p.destroy();
        }

        function test_stamp_parses_as_a_date() {
            var p = make(true);
            p.isoDate = "2026-02-29"; // 2026 is not a leap year
            verify(!isNaN(Date.parse(p.stamp)), p.stamp);
            p.destroy();
        }

        function test_a_start_before_an_end_orders_correctly() {
            var start = make(false);
            var end = make(true);
            start.isoDate = "2026-11-01";
            end.isoDate = "2026-11-01";
            // Same day: the end must still come after the start.
            verify(Date.parse(end.stamp) > Date.parse(start.stamp), start.stamp + " / " + end.stamp);
            start.destroy();
            end.destroy();
        }

        function test_iso_helper_pads_months_and_days() {
            var p = make(false);
            compare(p.isoOf(2026, 0, 1), "2026-01-01");
            compare(p.isoOf(2026, 10, 9), "2026-11-09");
            p.destroy();
        }

        function test_changed_carries_the_stamp() {
            var p = make(false);
            var seen = [];
            p.changed.connect(function (s) { seen.push(s); });
            p.isoDate = "2026-11-01";
            p.changed(p.stamp);
            compare(seen.length, 1);
            compare(seen[0], p.stamp);
            p.destroy();
        }
    }
}
