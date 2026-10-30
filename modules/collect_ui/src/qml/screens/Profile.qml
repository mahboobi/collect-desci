// Profile — how this person describes themselves to project members.
//
// Kept on this device. Each field has its own "Share with project members"
// switch; a shared field is encrypted separately for each member of each
// project this person is in, and goes over Logos Delivery — never Logos
// Storage. Unshared fields never leave the machine.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import collect
import "../components"

Flickable {
    id: screen

    /// `profile.get` data: `{ fields: { name: { value, share } }, sex_options }`.
    property var profile: null
    /// Problems from the last save, keyed by field.
    property var problems: ({})
    property bool busy: false
    property string savedNote: ""

    signal back()
    signal save(var fields)
    signal pickPhoto(string url, bool share)
    /// Ask the backend's native chooser for a photo (see nativePicker).
    signal requestPhotoPick(bool share)

    /// Whether the backend can open the system chooser. When it can, the
    /// QtQuick dialog is not used at all: opened from this sandboxed view it
    /// is parented to an offscreen window and strands the person outside
    /// basecamp.
    property bool nativePicker: false
    signal clearPhoto()

    // The edits in progress, keyed like `profile.fields`. Seeded from
    // `profile` whenever a fresh one arrives.
    property var draft: ({})

    onProfileChanged: screen.reseed()
    Component.onCompleted: screen.reseed()

    function reseed() {
        var d = {};
        var f = screen.profile && screen.profile.fields ? screen.profile.fields : {};
        for (var k in f)
            d[k] = { value: f[k].value === null ? "" : String(f[k].value), share: f[k].share === true };
        screen.draft = d;
    }

    function entry(name) {
        return screen.draft[name] ? screen.draft[name] : { value: "", share: false };
    }

    function setValue(name, value) {
        var d = Object.assign({}, screen.draft);
        d[name] = { value: value, share: screen.entry(name).share };
        screen.draft = d;
        screen.savedNote = "";
    }

    function setShare(name, share) {
        var d = Object.assign({}, screen.draft);
        d[name] = { value: screen.entry(name).value, share: share };
        screen.draft = d;
        screen.savedNote = "";
    }

    readonly property var sexLabels: ({
        female: "Female", male: "Male", intersex: "Intersex",
        other: "Other", prefer_not: "Prefer not to say"
    })

    readonly property int sharedCount: {
        var n = 0;
        for (var k in screen.draft)
            if (screen.draft[k].share && screen.draft[k].value !== "")
                n++;
        return n;
    }

    contentWidth: width
    contentHeight: body.implicitHeight + Theme.s48 * 2
    clip: true
    boundsBehavior: Flickable.StopAtBounds

    Column {
        id: body
        x: Theme.s32
        y: Theme.s32
        width: Math.min(screen.width - Theme.s32 * 2, 640)
        spacing: Theme.s24

        Button {
            id: backButton
            height: Theme.chipHeight
            leftPadding: Theme.s12
            rightPadding: Theme.s14
            text: "← Back"
            Accessible.name: "Back"
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

        Column {
            width: parent.width
            spacing: Theme.s8

            Text {
                text: "Science profile"
                font.family: Theme.sans
                font.pixelSize: Theme.pageTitle
                font.weight: Theme.semibold
                font.letterSpacing: Theme.titleSpacing
                color: Theme.ink
            }

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: "Kept on this device. Only fields you switch to “Share” are sent, "
                      + "encrypted so only the other members of your projects can read them. "
                      + "Nothing here is ever saved to Logos Storage."
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: Theme.ink2
                lineHeight: Theme.bodyLineHeight
                lineHeightMode: Text.ProportionalHeight
            }
        }

        // ---- photo ----
        Row {
            width: parent.width
            spacing: Theme.s20

            Avatar {
                size: 88
                photo: screen.profile && screen.profile.fields && screen.profile.fields.photo.value
                       ? screen.profile.fields.photo.value : ""
                name: screen.entry("alias").value
            }

            Column {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width - 88 - Theme.s20
                spacing: Theme.s8

                Row {
                    spacing: Theme.s10

                    Button {
                        id: choosePhoto
                        height: Theme.touchTarget
                        leftPadding: Theme.s20
                        rightPadding: Theme.s20
                        text: "Choose photo"
                        Accessible.name: "Choose a profile photo"
                        enabled: !screen.busy
                        onClicked: {
                            if (screen.nativePicker)
                                screen.requestPhotoPick(screen.entry("photo").share);
                            else if (pickerLoader.item)
                                pickerLoader.item.open();
                            else
                                pathRow.visible = true;
                        }
                        contentItem: Text {
                            text: choosePhoto.text
                            font.family: Theme.sans
                            font.pixelSize: Theme.body
                            color: Theme.ink
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                        background: Rectangle {
                            radius: Theme.rControl
                            color: choosePhoto.down ? Theme.surfaceSunken : Theme.surface
                            border.width: 1
                            border.color: Theme.inputBorder
                        }
                    }

                    Button {
                        id: removePhoto
                        visible: !!(screen.profile && screen.profile.fields && screen.profile.fields.photo.value)
                        height: Theme.touchTarget
                        text: "Remove"
                        Accessible.name: "Remove the profile photo"
                        onClicked: screen.clearPhoto()
                        contentItem: Text {
                            text: removePhoto.text
                            font.family: Theme.sans
                            font.pixelSize: Theme.body
                            color: Theme.ink2
                            verticalAlignment: Text.AlignVCenter
                        }
                        background: null
                    }
                }

                CheckRow {
                    width: parent.width
                    label: "Share photo with project members"
                    on: screen.entry("photo").share
                    onToggled: function (on) { screen.setShare("photo", on); }
                }

                Text {
                    width: parent.width
                    wrapMode: Text.Wrap
                    text: "Location and camera details are removed, and only a small square copy is kept."
                    font.family: Theme.sans
                    font.pixelSize: Theme.meta
                    color: Theme.ink3
                }

                // Shown when the sandbox has no file dialog.
                Row {
                    id: pathRow
                    visible: false
                    spacing: Theme.s8

                    LabelledInput {
                        id: pathInput
                        width: 320
                        label: "Path to an image file"
                        mono: true
                        hint: "This app cannot open a file picker here, so paste the file's full path."
                    }

                    Button {
                        id: usePath
                        anchors.bottom: parent.bottom
                        anchors.bottomMargin: pathInput.hint !== "" ? Theme.s24 : 0
                        height: Theme.touchTarget
                        text: "Use"
                        Accessible.name: "Use this image as the profile photo"
                        onClicked: screen.pickPhoto(pathInput.text, screen.entry("photo").share)
                        background: Rectangle {
                            radius: Theme.rControl
                            color: Theme.surface
                            border.width: 1
                            border.color: Theme.inputBorder
                        }
                    }
                }

                Text {
                    visible: !!screen.problems.photo
                    width: parent.width
                    wrapMode: Text.Wrap
                    text: screen.problems.photo ? screen.problems.photo : ""
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    color: Theme.amber
                }
            }
        }

        Loader {
            id: pickerLoader
            active: !screen.nativePicker
            source: "../components/PhotoPicker.qml"
            onStatusChanged: {
                if (status === Loader.Error)
                    console.warn("collect: no file dialog in this sandbox; offering a typed path");
            }
        }

        Connections {
            target: pickerLoader.item
            ignoreUnknownSignals: true
            function onPicked(url) { screen.pickPhoto(url, screen.entry("photo").share); }
        }

        // ---- text fields ----
        Repeater {
            model: [
                { key: "alias",       label: "Name or alias",        hint: "How other members see you in discussions.", mono: false, max: 80 },
                { key: "affiliation", label: "Affiliation",          hint: "Institution, lab, school or group.",        mono: false, max: 120 },
                { key: "location",    label: "Location",             hint: "A city or region. Not a GPS position.",       mono: false, max: 120 },
                { key: "age",         label: "Age",                  hint: "",                                            mono: false, max: 3 },
                { key: "email",       label: "Email",                hint: "",                                            mono: false, max: 254 },
                { key: "phone",       label: "Phone",                hint: "",                                            mono: false, max: 32 },
                { key: "wallet",      label: "Wallet address",       hint: "Only a public address. Never paste a seed phrase or private key.", mono: true, max: 200 }
            ]

            delegate: Column {
                id: fieldBlock
                required property var modelData
                width: body.width
                spacing: Theme.s6

                LabelledInput {
                    width: parent.width
                    label: fieldBlock.modelData.label
                    hint: fieldBlock.modelData.hint
                    mono: fieldBlock.modelData.mono
                    maximumLength: fieldBlock.modelData.max
                    text: screen.entry(fieldBlock.modelData.key).value
                    inputHints: fieldBlock.modelData.key === "age" ? Qt.ImhDigitsOnly
                              : fieldBlock.modelData.key === "email" ? Qt.ImhEmailCharactersOnly
                              : fieldBlock.modelData.key === "phone" ? Qt.ImhDialableCharactersOnly
                              : Qt.ImhNone
                    onEdited: function (value) { screen.setValue(fieldBlock.modelData.key, value); }
                }

                Text {
                    visible: !!screen.problems[fieldBlock.modelData.key]
                    width: parent.width
                    wrapMode: Text.Wrap
                    text: screen.problems[fieldBlock.modelData.key] ? screen.problems[fieldBlock.modelData.key] : ""
                    font.family: Theme.sans
                    font.pixelSize: Theme.small
                    color: Theme.amber
                }

                CheckRow {
                    width: parent.width
                    label: "Share with project members"
                    on: screen.entry(fieldBlock.modelData.key).share
                    onToggled: function (on) { screen.setShare(fieldBlock.modelData.key, on); }
                }
            }
        }

        // ---- sex ----
        Column {
            width: parent.width
            spacing: Theme.s8

            Text {
                text: "Sex"
                font.family: Theme.sans
                font.pixelSize: Theme.meta
                font.weight: Theme.medium
                color: Theme.ink2
            }

            Flow {
                width: parent.width
                spacing: Theme.s8

                Repeater {
                    model: screen.profile && screen.profile.sex_options ? screen.profile.sex_options : []

                    delegate: Button {
                        id: sexChip
                        required property string modelData
                        readonly property bool picked: screen.entry("sex").value === modelData
                        height: Theme.touchTarget
                        leftPadding: Theme.s14
                        rightPadding: Theme.s14
                        text: screen.sexLabels[modelData] ? screen.sexLabels[modelData] : modelData
                        Accessible.name: "Sex: " + text
                        Accessible.checkable: true
                        Accessible.checked: picked
                        // Clicking the picked one clears it: blank is allowed.
                        onClicked: screen.setValue("sex", picked ? "" : modelData)
                        contentItem: Text {
                            text: sexChip.text
                            font.family: Theme.sans
                            font.pixelSize: Theme.small
                            color: sexChip.picked ? Theme.textOnAccent : Theme.ink
                            verticalAlignment: Text.AlignVCenter
                        }
                        background: Rectangle {
                            radius: Theme.rPill
                            color: sexChip.picked ? Theme.accent : Theme.surface
                            border.width: sexChip.picked ? 0 : 1
                            border.color: Theme.inputBorder
                        }
                    }
                }
            }

            Text {
                visible: !!screen.problems.sex
                text: screen.problems.sex ? screen.problems.sex : ""
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: Theme.amber
            }

            CheckRow {
                width: parent.width
                label: "Share with project members"
                on: screen.entry("sex").share
                onToggled: function (on) { screen.setShare("sex", on); }
            }
        }

        // ---- save ----
        Column {
            width: parent.width
            spacing: Theme.s10

            Text {
                width: parent.width
                wrapMode: Text.Wrap
                text: screen.savedNote !== "" ? screen.savedNote
                      : screen.sharedCount === 0 ? "Nothing is shared. Other members see only your key."
                      : screen.sharedCount + (screen.sharedCount === 1 ? " field is" : " fields are")
                        + " shared with the members of your projects."
                font.family: Theme.sans
                font.pixelSize: Theme.small
                color: screen.savedNote !== "" ? Theme.accent : Theme.ink2
            }

            Button {
                id: saveButton
                height: Theme.touchTarget
                leftPadding: Theme.s28
                rightPadding: Theme.s28
                enabled: !screen.busy
                text: screen.busy ? "Saving…" : "Save profile"
                Accessible.name: "Save profile"
                onClicked: {
                    var out = {};
                    for (var k in screen.draft)
                        out[k] = { value: screen.draft[k].value, share: screen.draft[k].share };
                    screen.save(out);
                }
                contentItem: Text {
                    text: saveButton.text
                    font.family: Theme.sans
                    font.pixelSize: Theme.body
                    font.weight: Theme.medium
                    color: Theme.textOnAccent
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle {
                    radius: Theme.rControl
                    color: saveButton.down ? Theme.accentHover : Theme.accent
                }
            }
        }
    }
}
