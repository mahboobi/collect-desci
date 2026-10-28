#!/usr/bin/env bash
# Render the Collect screens outside Basecamp.
#
# Opens a window showing each screen against the example documents. For
# automated checks use tests/run.sh instead.
#
# Uses the Qt 6 already in the nix store; override with QT_QML_ROOT.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
qtd="${QT_QML_ROOT:-$(ls -d /nix/store/*-qtdeclarative-6.*/ 2>/dev/null | head -1)}"
qtb="$(ls -d /nix/store/*-qtbase-6.*/ 2>/dev/null | grep -v -- -dev | grep -v only-plugins | head -1)"

if [ -z "$qtd" ] || [ ! -x "$qtd/bin/qml" ]; then
  echo "no Qt 6 qml runtime found; set QT_QML_ROOT to a qtdeclarative prefix" >&2
  exit 1
fi

import_paths="-I $here/../src/qml -I $qtd/lib/qt-6/qml -I $qtb/lib/qt-6/qml"

# The harness reads examples/*.json off disk to seed Core. Qt blocks local-file
# XHR unless asked. The module itself never does this — it has no filesystem.
export QML_XHR_ALLOW_FILE_READ=1

# The design system customizes every control, which the macOS native style
# refuses to allow. Basecamp's own shell has to do the same.
export QT_QUICK_CONTROLS_STYLE=Basic

exec "$qtd/bin/qml" $import_paths "$here/Harness.qml"
