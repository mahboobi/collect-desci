// The native file dialog, kept in its own file and loaded through a Loader.
//
// Basecamp sandboxes UI modules, and whether QtQuick.Dialogs is reachable
// from inside that sandbox is not documented (SPEC Q13). If it is not, only
// this file fails to load — the profile screen notices `status` and offers a
// typed path instead, rather than the whole screen failing.
import QtQuick
import QtQuick.Dialogs

FileDialog {
    id: dialog

    signal picked(string url)

    title: "Choose a profile photo"
    nameFilters: ["Images (*.jpg *.jpeg *.png *.webp)"]
    fileMode: FileDialog.OpenFile
    onAccepted: dialog.picked(dialog.selectedFile.toString())
}
