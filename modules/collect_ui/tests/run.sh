#!/usr/bin/env bash
# Run the QML tests for collect_ui.
#
# Uses the Qt 6 already in the nix store; override with QT_QML_ROOT.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The Qt Basecamp ships (6.9.2 in Basecamp 0.3.2), not whichever is first in
# the store: a newer Qt accepts QML that Basecamp refuses, which is how
# `function short()` — a reserved word in 6.9 — passed here and failed there.
QT_VERSION="${QT_VERSION:-6.9.2}"
qtd="${QT_QML_ROOT:-$(ls -d /nix/store/*-qtdeclarative-${QT_VERSION}/ 2>/dev/null | head -1)}"
qtb="$(ls -d /nix/store/*-qtbase-${QT_VERSION}/ 2>/dev/null | head -1)"

if [ -z "$qtd" ] || [ ! -x "$qtd/bin/qmltestrunner" ]; then
  echo "no Qt 6 qmltestrunner found; set QT_QML_ROOT to a qtdeclarative prefix" >&2
  exit 1
fi

# offscreen + software so this runs headless in CI and steals no focus.
# Basic style because the design system customizes every control, which the
# macOS native style refuses.
export QT_QPA_PLATFORM=offscreen
export QT_QUICK_BACKEND=software
export QT_QUICK_CONTROLS_STYLE=Basic

exec "$qtd/bin/qmltestrunner" \
  -import "$here/../src/qml" \
  -import "$qtd/lib/qt-6/qml" \
  -import "$qtb/lib/qt-6/qml" \
  -input "$here"
