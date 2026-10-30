// Graph paper: faint minor lines every `step` pixels and a stronger line
// every `majorEvery` of them. Drawn behind the whole module and inside empty
// image areas, so the app reads as a lab notebook.
//
// Decoration only: invisible to accessibility, and it takes no input.
import QtQuick
import collect

Canvas {
    id: paper

    property int step: Theme.gridStep
    property int majorEvery: Theme.gridMajorEvery
    property color minorColor: Theme.gridMinor
    property color majorColor: Theme.gridMajor

    Accessible.ignored: true
    enabled: false

    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
    onStepChanged: requestPaint()
    onMinorColorChanged: requestPaint()
    onMajorColorChanged: requestPaint()

    onPaint: {
        var c = getContext("2d");
        c.reset();
        c.lineWidth = 1;
        // Minor first, so a major line is never painted over by a faint one.
        for (var pass = 0; pass < 2; pass++) {
            var major = pass === 1;
            c.strokeStyle = major ? paper.majorColor : paper.minorColor;
            c.beginPath();
            var i, at;
            for (i = 0, at = 0.5; at < width; i++, at += paper.step) {
                if ((i % paper.majorEvery === 0) === major) {
                    c.moveTo(at, 0);
                    c.lineTo(at, height);
                }
            }
            for (i = 0, at = 0.5; at < height; i++, at += paper.step) {
                if ((i % paper.majorEvery === 0) === major) {
                    c.moveTo(0, at);
                    c.lineTo(width, at);
                }
            }
            c.stroke();
        }
    }
}
