// The nine field types, and a contract-valid starting config for each.
//
// `.pragma library` so every screen shares one copy.
//
// The defaults matter: a `choice` needs at least two options and a `location`
// needs methods and a store rule, so a field added without them would make
// the whole protocol unsaveable. Adding a field should never put the form in
// a state the creator has to repair before they can save.
.pragma library

var KINDS = [
    { type: "text",     label: "Short text",   hint: "One line." },
    { type: "longtext", label: "Long text",    hint: "A paragraph." },
    { type: "number",   label: "Number",       hint: "A count or measurement." },
    { type: "choice",   label: "Choose one",   hint: "Radio buttons." },
    { type: "multi",    label: "Choose many",  hint: "Checkboxes." },
    { type: "datetime", label: "Date and time", hint: "With a UTC offset." },
    { type: "photo",    label: "Photos",       hint: "Metadata is removed before upload." },
    { type: "location", label: "Location",     hint: "Rounded to 1 km by default." },
    { type: "yesno",    label: "Yes or no",    hint: "Two options." }
];

function labelFor(type) {
    for (var i = 0; i < KINDS.length; i++) {
        if (KINDS[i].type === type)
            return KINDS[i].label;
    }
    return type;
}

/// A config that already satisfies the contract for this type.
function defaultConfig(type) {
    switch (type) {
    case "text":     return { max_length: 200 };
    case "longtext": return { max_length: 5000 };
    case "number":   return { integer: true };
    case "choice":
    case "multi":    return { options: [
                        { value: "option_one", label: "Option one" },
                        { value: "option_two", label: "Option two" }
                     ] };
    case "datetime": return { fill_from: ["now"], no_future: true };
    case "photo":    return { min: 1, max: 3, max_mb_each: 25,
                              fill_from_exif: ["location", "datetime"] };
    // rounded_1km by default: all data is public and nothing on Storage can
    // be deleted, so the safer rule is the one that should need a decision to
    // leave, not a decision to reach.
    case "location": return { methods: ["manual", "photo"], store: "rounded_1km" };
    default:         return {};
    }
}

/// A field key from a label: a lower-case letter, then letters, digits and
/// underscores, which is the contract's pattern.
function keyFrom(label, taken) {
    var out = "";
    for (var i = 0; i < label.length; i++) {
        var c = label[i].toLowerCase();
        if (c >= "a" && c <= "z")
            out += c;
        else if (out.length > 0 && ((c >= "0" && c <= "9") || c === "_" || c === " "))
            out += (c === " " ? "_" : c);
    }
    out = out.replace(/_+/g, "_").replace(/^_+|_+$/g, "").slice(0, 40);
    if (out === "" || out[0] < "a" || out[0] > "z")
        out = "field";
    var candidate = out;
    var n = 2;
    while (taken.indexOf(candidate) !== -1) {
        var suffix = "_" + n;
        candidate = out.slice(0, 40 - suffix.length) + suffix;
        n += 1;
    }
    return candidate;
}

function newField(type, taken) {
    var label = labelFor(type);
    return {
        key: keyFrom(label, taken),
        type: type,
        label: label,
        required: false,
        config: defaultConfig(type)
    };
}
