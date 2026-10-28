// One field: its label, the required/optional marker, help text, the delegate
// itself, and any complaint.
//
// The delegate is chosen by `field.type` and nothing here knows what a
// wildlife sighting is — CLAUDE.md: the form is rendered from the protocol
// JSON, never hard-coded.
pragma ComponentBehavior: Bound

import QtQuick
import collect

Column {
    id: row

    required property var field
    // Set once the submitter has tried to submit, so an untouched form is not
    // covered in red before anyone has done anything.
    property bool showProblems: false

    readonly property var delegateItem: loader.item
    // True when field.type has no delegate in this build.
    readonly property bool unsupported: sourceFor(field.type) === ""
    readonly property bool isBlank: delegateItem ? delegateItem.blank : true
    readonly property string problem: delegateItem ? delegateItem.problem : ""
    readonly property bool missing: field.required && isBlank
    readonly property bool satisfied: !missing && problem === ""
    readonly property var value: delegateItem ? delegateItem.value : undefined

    width: parent ? parent.width : 0
    spacing: Theme.s8

    function sourceFor(type) {
        switch (type) {
        case "text":
        case "longtext":  return "../fields/TextInputField.qml";
        case "number":    return "../fields/NumberField.qml";
        case "choice":    return "../fields/ChoiceField.qml";
        case "multi":     return "../fields/MultiField.qml";
        case "datetime":  return "../fields/DateTimeField.qml";
        case "photo":     return "../fields/PhotoField.qml";
        case "location":  return "../fields/LocationField.qml";
        case "yesno":     return "../fields/YesNoField.qml";
        // A type this build does not know is not rendered as a text box: that
        // would silently collect the wrong shape. The contract caps the set,
        // so this only fires against a newer protocol.
        default:          return "";
        }
    }

    // ---- label line ----
    Row {
        spacing: Theme.s8

        Text {
            id: labelText
            text: row.field.label
            font.family: Theme.sans
            font.pixelSize: Theme.sectionTitle
            font.weight: Theme.semibold
            color: Theme.ink
        }

        Text {
            anchors.baseline: labelText.baseline
            text: "\u00b7 " + (row.field.required ? "required" : "optional")
                  + (row.field.type === "photo" && row.field.config
                     ? row.photoRange(row.field.config) : "")
            font.family: Theme.sans
            font.pixelSize: Theme.meta
            color: row.showProblems && row.missing ? Theme.amber : Theme.ink3
        }
    }

    function photoRange(config) {
        if (config.min !== undefined && config.max !== undefined)
            return " \u00b7 " + config.min + " to " + config.max;
        if (config.max !== undefined)
            return " \u00b7 up to " + config.max;
        return "";
    }

    Text {
        visible: !!row.field.help
        width: parent.width
        wrapMode: Text.Wrap
        text: row.field.help ? row.field.help : ""
        font.family: Theme.sans
        font.pixelSize: Theme.small
        color: Theme.ink2
    }

    // ---- the delegate ----
    //
    // setSource rather than a `source` binding: `field` is a required
    // property, so it has to be supplied at creation. Assigning it in
    // onLoaded is too late and the delegate never constructs.
    Loader {
        id: loader
        width: parent.width

        Component.onCompleted: {
            var url = row.sourceFor(row.field.type);
            if (url !== "")
                loader.setSource(url, { field: row.field });
        }
    }

    Text {
        visible: row.unsupported
        width: parent.width
        wrapMode: Text.Wrap
        text: "This form uses a field type this version of Collect does not know ("
              + row.field.type + "). Update Collect to fill it in."
        font.family: Theme.sans
        font.pixelSize: Theme.small
        color: Theme.amber
    }

    // ---- the complaint ----
    Text {
        visible: row.showProblems && (row.problem !== "" || row.missing)
        width: parent.width
        wrapMode: Text.Wrap
        text: row.problem !== "" ? row.problem : "This field is required"
        font.family: Theme.sans
        font.pixelSize: Theme.small
        color: Theme.amber
    }
}
