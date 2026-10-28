// Rendering one answer as a table cell.
//
// Each field type stores a different shape, so a single toString would show
// "[object Object]" for half the columns. The type comes down with the column
// from collect_core, so the cell knows what it is looking at.
.pragma library

function render(type, value, options) {
    if (value === undefined || value === null)
        return "";

    switch (type) {
    case "photo":
        // The cell cannot show the photo without fetching it, so say how many
        // and leave opening one to the row.
        return value.length === 1 ? "1 photo" : value.length + " photos";

    case "location":
        if (value.lat === undefined || value.lon === undefined)
            return "";
        var place = Number(value.lat).toFixed(value.rounded ? 3 : 5)
                  + ", " + Number(value.lon).toFixed(value.rounded ? 3 : 5);
        // Whether it was rounded changes what the number means, so it travels
        // with it rather than being dropped.
        return place + (value.rounded ? " (1 km)" : "");

    case "multi":
        return value.map(function (v) { return labelOf(options, v); }).join(", ");

    case "choice":
        return labelOf(options, value);

    case "yesno":
        return value ? "Yes" : "No";

    case "datetime":
        // Keep the offset: it is part of what was recorded.
        return String(value).replace("T", " ");

    case "longtext":
        var text = String(value);
        return text.length > 60 ? text.slice(0, 60) + "\u2026" : text;

    default:
        return String(value);
    }
}

/// An option's label, falling back to the stored value.
///
/// A value with no label means the option was removed from a later protocol
/// version — showing the raw value is better than showing nothing, because
/// the submission still holds it.
function labelOf(options, value) {
    if (!options)
        return String(value);
    for (var i = 0; i < options.length; i++) {
        if (options[i].value === value)
            return options[i].label;
    }
    return String(value);
}
