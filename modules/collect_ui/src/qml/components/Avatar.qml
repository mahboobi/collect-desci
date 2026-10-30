// A round avatar: the person's photo if we have one, otherwise the first
// letter of their name, otherwise the first two characters of their key.
//
// `photo` is only ever a `data:image/jpeg;base64,` URI that collect_core
// produced or re-checked (profile::is_shared_photo). The QML sandbox has no
// network and no files, so nothing else could be shown here anyway.
import QtQuick
import collect

Rectangle {
    id: avatar

    property string photo: ""
    property string name: ""
    property string fallback: ""
    property int size: 36

    width: size
    height: size
    radius: size / 2
    color: Theme.accentTint
    clip: true

    Image {
        id: picture
        anchors.fill: parent
        visible: avatar.photo !== "" && status === Image.Ready
        source: avatar.photo.indexOf("data:image/jpeg;base64,") === 0 ? avatar.photo : ""
        fillMode: Image.PreserveAspectCrop
        smooth: true
    }

    Text {
        anchors.centerIn: parent
        visible: !picture.visible
        text: avatar.name !== "" ? avatar.name.charAt(0).toUpperCase()
                                 : avatar.fallback.substring(0, 2)
        font.family: avatar.name !== "" ? Theme.sans : Theme.mono
        font.pixelSize: Math.round(avatar.size * 0.42)
        font.weight: Theme.semibold
        color: Theme.accent
    }
}
