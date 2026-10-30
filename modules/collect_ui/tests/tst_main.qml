// Main.qml must compile. The screen tests load screens one at a time, so an
// error in Main.qml itself — a second `onReadyChanged` handler, which QML
// reports as "Property value set multiple times" — passed every test and
// only showed up as Basecamp refusing to open Collect.
import QtQuick
import QtTest

TestCase {
    name: "Main"

    function test_main_compiles() {
        var component = Qt.createComponent(Qt.resolvedUrl("../src/qml/Main.qml"));
        // Compiling only: creating it would need Basecamp's `logos` object.
        compare(component.status, Component.Ready, component.errorString());
    }

    // Every other file too: a screen without its own test would otherwise
    // only fail when someone opens it inside Basecamp.
    function test_every_file_compiles() {
        var files = [
            "components/Avatar.qml",
            "components/Badge.qml",
            "components/CheckRow.qml",
            "components/DatePicker.qml",
            "components/ExperimentCard.qml",
            "components/FieldRow.qml",
            "components/FieldSettings.qml",
            "components/FormRenderer.qml",
            "components/GridPaper.qml",
            "components/LabelledInput.qml",
            "components/PhotoPicker.qml",
            "components/PublicNotice.qml",
            "components/Thumbnail.qml",
            "fields/ChoiceField.qml",
            "fields/DateTimeField.qml",
            "fields/FieldBase.qml",
            "fields/LocationField.qml",
            "fields/MultiField.qml",
            "fields/NumberField.qml",
            "fields/PhotoField.qml",
            "fields/TextInputField.qml",
            "fields/YesNoField.qml",
            "screens/Builder.qml",
            "screens/Discussion.qml",
            "screens/Form.qml",
            "screens/Home.qml",
            "screens/Join.qml",
            "screens/NewExperiment.qml",
            "screens/Profile.qml",
            "screens/Results.qml",
            "screens/Submitted.qml"
        ];
        for (var i = 0; i < files.length; i++) {
            var component = Qt.createComponent(Qt.resolvedUrl("../src/qml/" + files[i]));
            compare(component.status, Component.Ready, files[i] + ": " + component.errorString());
        }
    }
}
