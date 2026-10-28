// The whole form for one protocol, in protocol order.
//
// Holds the cross-field wiring the spec calls for in section 5 step 3: a photo
// that carried GPS or a timestamp prefills `location` and `datetime`, but only
// where the protocol says it may.
pragma ComponentBehavior: Bound

import QtQuick
import collect

Column {
    id: renderer

    required property var protocol
    property bool showProblems: false

    readonly property var fields: protocol && protocol.fields ? protocol.fields : []

    // Repeater.itemAt() is typed QQuickItem, so assert the row type in one
    // place rather than at every call site.
    function rowAt(i) {
        return rows.itemAt(i) as FieldRow;
    }

    // Every field that is required and unanswered.
    readonly property var missingLabels: {
        var out = [];
        for (var i = 0; i < rows.count; i++) {
            var r = renderer.rowAt(i);
            if (r && r.missing)
                out.push(r.field.label);
        }
        return out;
    }

    readonly property bool complete: {
        for (var i = 0; i < rows.count; i++) {
            var r = renderer.rowAt(i);
            if (r && !r.satisfied)
                return false;
        }
        return rows.count > 0;
    }

    // `values` as it would be signed: only answered fields, keyed by field key.
    function values() {
        var out = {};
        for (var i = 0; i < rows.count; i++) {
            var r = renderer.rowAt(i);
            if (r && r.value !== undefined)
                out[r.field.key] = r.value;
        }
        return out;
    }

    function focusFirstMissing() {
        for (var i = 0; i < rows.count; i++) {
            var r = renderer.rowAt(i);
            if (r && !r.satisfied && r.delegateItem) {
                r.delegateItem.focusField();
                return;
            }
        }
    }

    // Hand a processed photo's EXIF to the fields allowed to take it.
    function applyPhotoExif(exif, name) {
        for (var i = 0; i < rows.count; i++) {
            var r = renderer.rowAt(i);
            if (!r || !r.delegateItem)
                continue;
            var f = r.field;
            var allowed = f.config && f.config.fill_from_exif ? f.config.fill_from_exif : [];
            if (f.type === "location" && exif.location)
                r.delegateItem.fillFromPhoto(exif.location);
            if (f.type === "datetime" && exif.datetime)
                r.delegateItem.fillFromPhoto(exif.datetime, name);
        }
    }

    width: parent ? parent.width : 0
    spacing: Theme.s28

    Repeater {
        id: rows
        model: renderer.fields

        delegate: FieldRow {
            required property var modelData
            field: modelData
            showProblems: renderer.showProblems

            // A photo field announces what collect_core read from the file.
            Component.onCompleted: {
                if (modelData.type === "photo" && delegateItem)
                    delegateItem.exifRead.connect(renderer.applyPhotoExif);
            }
        }
    }
}
