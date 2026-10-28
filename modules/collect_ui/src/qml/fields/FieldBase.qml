// What every field delegate exposes to the renderer.
//
// `blank` and `problem` are kept apart on purpose, matching
// collect_core::fields: an untouched field reads as unanswered ("required"),
// not as an error. Only a field someone has actually filled in badly shows a
// problem.
import QtQuick

Item {
    id: base

    // The field object straight out of the protocol JSON.
    required property var field
    readonly property var config: field && field.config ? field.config : ({})

    // The value as it would be stored in a submission. `undefined` until
    // answered.
    property var value: undefined

    // Nothing entered yet.
    property bool blank: value === undefined || value === null || value === ""

    // A local, immediate complaint. Empty string means nothing to say.
    //
    // This is for responsiveness only — it decides whether Submit looks
    // enabled. collect_core::fields::validate_values is authoritative and runs
    // again on submit, and the creator runs it a third time on receive. Keep
    // this layer thin: a rule implemented here as well as there is a rule that
    // can drift.
    property string problem: ""

    // Lets the renderer put focus on the first unanswered field.
    function focusField() {}

    implicitHeight: childrenRect.height
    width: parent ? parent.width : 0
}
