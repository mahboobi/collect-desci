// Theme.qml — Open Labbook design tokens as a QML singleton.
// Register with a qmldir line:  singleton Theme 1.0 Theme.qml
// Source of truth: design/tokens.json (keep both in sync).
pragma Singleton
import QtQuick

QtObject {
    // ---- Neutrals
    readonly property color ground: "#F5F6F3"
    readonly property color surface: "#FFFFFF"
    readonly property color surfaceSunken: "#ECEEE9"
    readonly property color line: "#DDE0DA"
    readonly property color lineSoft: "#ECEEE9"
    readonly property color inputBorder: "#C9CDC6"
    readonly property color dashed: "#B9BEB6"
    readonly property color ink: "#15181C"
    readonly property color ink2: "#4C535C"
    readonly property color ink3: "#646B73"
    readonly property color onAccent: "#FFFFFF"

    // ---- Semantic
    readonly property color accent: "#0B6E4F"        // done, verified, anchored, primary action
    readonly property color accentHover: "#08523B"
    readonly property color accentTint: "#E3F1EB"
    readonly property color accentTintSoft: "#F3F9F6" // selected radio/checkbox card fill
    readonly property color violet: "#4B3FB0"        // current, committed, hashes, branches
    readonly property color violetTint: "#ECEAFB"
    readonly property color amber: "#8A4B00"         // attention, waiting, single copy
    readonly property color amberTint: "#FBEFD9"
    readonly property color slate: "#3D4A5C"         // neutral, draft, negative result
    readonly property color slateTint: "#E6EAF0"
    readonly property color backdrop: Qt.rgba(21 / 255, 24 / 255, 28 / 255, 0.48)
    readonly property color modalShadow: Qt.rgba(21 / 255, 24 / 255, 28 / 255, 0.24)

    // ---- Fonts (load the bundled .ttf files with FontLoader and set these names)
    property string sans: "IBM Plex Sans"
    property string mono: "IBM Plex Mono"

    // ---- Type sizes (px)
    readonly property int pageTitle: 28
    readonly property int modalTitle: 20
    readonly property int cardTitle: 18
    readonly property int metric: 22
    readonly property int sectionTitle: 16
    readonly property int lede: 15
    readonly property int body: 14
    readonly property int small: 13
    readonly property int meta: 12
    readonly property int overline: 11
    readonly property int stageLabel: 11
    readonly property int fingerprint: 14
    readonly property int code: 22
    readonly property real bodyLineHeight: 1.45
    readonly property real overlineSpacing: 0.66   // 0.06em at 11 px
    readonly property real titleSpacing: -0.28     // -0.01em at 28 px

    // ---- Weights
    readonly property int regular: Font.Normal     // 400
    readonly property int medium: Font.Medium      // 500
    readonly property int semibold: Font.DemiBold  // 600

    // ---- Spacing
    readonly property int s2: 2
    readonly property int s4: 4
    readonly property int s6: 6
    readonly property int s8: 8
    readonly property int s10: 10
    readonly property int s12: 12
    readonly property int s14: 14
    readonly property int s16: 16
    readonly property int s20: 20
    readonly property int s24: 24
    readonly property int s28: 28
    readonly property int s32: 32
    readonly property int s48: 48

    // ---- Radius
    readonly property int rProgress: 3
    readonly property int rLogo: 7
    readonly property int rControl: 8
    readonly property int rInner: 10
    readonly property int rPanel: 12
    readonly property int rIdeaCard: 14
    readonly property int rModal: 16
    readonly property int rPill: 999

    // ---- Sizes
    readonly property int touchTarget: 44
    readonly property int pillHeight: 40
    readonly property int chipHeight: 32
    readonly property int navWidth: 232
    readonly property int modalWidth: 640
    readonly property int stageBar: 6
    readonly property int stepBar: 4
    readonly property int statusDot: 8
    readonly property int checkbox: 18
    readonly property int ideaCardMin: 460
    readonly property int narrowBreakpoint: 900

    // ---- Badge variants: Theme.badge.done.fg etc.
    readonly property var badge: ({
        done:      { fg: "#0B6E4F", bg: "#E3F1EB" },
        current:   { fg: "#4B3FB0", bg: "#ECEAFB" },
        attention: { fg: "#8A4B00", bg: "#FBEFD9" },
        neutral:   { fg: "#3D4A5C", bg: "#E6EAF0" }
    })
}
