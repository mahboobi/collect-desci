// `location`. The rounding rule is the part that matters: under
// `rounded_1km` the exact point must never leave the machine, so the notice
// says which of the two is about to become public.
pragma ComponentBehavior: Bound

import QtQuick
import collect

FieldBase {
    id: root

    readonly property var methods: config.methods ? config.methods : []
    readonly property var maxAccuracy: config.max_accuracy_m
    readonly property bool rounds: config.store === "rounded_1km"

    property var lat: undefined
    property var lon: undefined
    property var accuracy: undefined
    property string method: ""

    blank: lat === undefined || lon === undefined || method === ""
    value: blank ? undefined : rounded()

    problem: {
        if (blank)
            return "";
        if (lat < -90 || lat > 90)
            return "Latitude is out of range";
        if (lon < -180 || lon > 180)
            return "Longitude is out of range";
        if (methods.indexOf(method) === -1)
            return "This form does not accept a location captured that way";
        if (maxAccuracy !== undefined && maxAccuracy !== null) {
            if (accuracy === undefined || accuracy === null)
                return "Needs an accuracy within " + maxAccuracy + " m";
            if (accuracy > maxAccuracy)
                return "Accuracy " + accuracy + " m is worse than " + maxAccuracy + " m";
        }
        return "";
    }

    function focusField() { /* the first coordinate input takes focus from the renderer */ }

    // Mirrors collect_core::geo::round_1km — the centre of the 0.01 degree
    // cell. Shown here only so the submitter can see what will be stored;
    // collect_core rounds again before anything is signed, and it is the one
    // that counts.
    function cellCentre(v) {
        var cell = Math.floor(v * 100);
        if ((cell + 1) / 100 <= v)
            cell += 1;
        else if (cell / 100 > v)
            cell -= 1;
        return (cell + 0.5) / 100;
    }

    function rounded() {
        var out = {
            lat: root.rounds ? cellCentre(root.lat) : root.lat,
            lon: root.rounds ? cellCentre(root.lon) : root.lon,
            method: root.method,
            rounded: root.rounds
        };
        if (root.accuracy !== undefined && root.accuracy !== null)
            out.accuracy_m = root.accuracy;
        return out;
    }

    // Called when a photo carried GPS and the protocol allows `photo`.
    function fillFromPhoto(exifLocation) {
        if (!exifLocation || root.methods.indexOf("photo") === -1)
            return;
        root.lat = exifLocation.lat;
        root.lon = exifLocation.lon;
        root.accuracy = exifLocation.accuracy_m;
        root.method = "photo";
    }

    Column {
        width: parent.width
        spacing: Theme.s10

        // Placeholder for the map (SPEC Q5: tiles need network, which the QML
        // sandbox does not have).
        Rectangle {
            width: parent.width
            height: 120
            radius: Theme.rInner
            color: Theme.surfaceSunken
            border.width: 1
            border.color: Theme.line

            Text {
                anchors.centerIn: parent
                text: "Schematic \u00b7 the app draws real map tiles"
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: Theme.ink3
            }
        }

        Row {
            spacing: Theme.s12

            Repeater {
                model: [{ key: "lat", label: "Latitude" }, { key: "lon", label: "Longitude" }]

                delegate: Column {
                    id: axis
                    required property var modelData
                    spacing: Theme.s4

                    Text {
                        text: axis.modelData.label
                        font.family: Theme.sans
                        font.pixelSize: Theme.meta
                        font.weight: Theme.medium
                        color: Theme.ink2
                    }

                    Rectangle {
                        width: 150
                        height: Theme.touchTarget
                        radius: Theme.rControl
                        color: Theme.surface
                        border.width: 1
                        border.color: Theme.inputBorder

                        TextInput {
                            id: coordInput
                            objectName: axis.modelData.key
                            anchors.fill: parent
                            anchors.margins: Theme.s10
                            verticalAlignment: TextInput.AlignVCenter
                            font.family: Theme.mono
                            font.pixelSize: Theme.body
                            color: Theme.ink
                            selectByMouse: true
                            inputMethodHints: Qt.ImhFormattedNumbersOnly
                            Accessible.name: axis.modelData.label + " for " + root.field.label
                            onTextEdited: {
                                var n = Number(text);
                                // Cleared or not a number: forget the old
                                // value, so a stale one cannot be submitted.
                                if (text === "" || isNaN(n))
                                    n = undefined;
                                if (axis.modelData.key === "lat")
                                    root.lat = n;
                                else
                                    root.lon = n;
                                // Typed by hand, so that is the method — if
                                // the protocol allows it.
                                if (root.methods.indexOf("manual") !== -1)
                                    root.method = "manual";
                            }
                        }
                    }
                }
            }
        }

        // What will actually be stored, and how precise it is. Not shown for
        // a position that will not be stored at all.
        Column {
            visible: !root.blank && root.problem === ""
            width: parent.width
            spacing: Theme.s4

            Text {
                text: root.rounds
                      ? "Stored as " + root.cellCentre(root.lat).toFixed(3) + ", "
                        + root.cellCentre(root.lon).toFixed(3) + " \u2014 rounded to about 1 km"
                      : "Stored as " + Number(root.lat).toFixed(5) + ", " + Number(root.lon).toFixed(5)
                font.family: Theme.mono
                font.pixelSize: Theme.small
                color: Theme.ink
            }

            Text {
                text: root.rounds
                      ? "Your exact position is never saved."
                      : "This location will be public."
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                color: root.rounds ? Theme.accent : Theme.amber
            }
        }
    }

}
