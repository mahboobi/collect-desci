#!/usr/bin/env bash
# Build, install and launch Collect in Logos Basecamp.
#
# Does not need `lgs`. Basecamp and lgpm are already in the Nix store from a
# previous `lgs basecamp setup`, and this script uses that exact pair — the
# alignment matters, see VARIANTS below.
#
#   tools/load.sh                 # profile "creator"
#   tools/load.sh alice           # any profile name
#   tools/load.sh creator --fresh # wipe the profile's installed modules first
#
# VARIANTS — the thing that cost the most time to work out:
#
#   .#lgx           -> variant darwin-arm64-dev
#   .#lgx-portable  -> variant darwin-arm64
#
# The scaffold-pinned basecamp (packages.app, bin/LogosBasecamp) and the
# pinned lgpm both speak **darwin-arm64-dev**, so `.#lgx` is correct here.
# The 0.3.0 tag's `bin-macos-app` is the opposite: it accepts only
# [darwin-arm64, darwin-aarch64] and refuses a -dev module with
# "not supported on this platform and will not be loadable". Mixing the two
# fails in both directions, and lgpm's own --platform flag does not override
# it in this dev build.
set -euo pipefail

PROFILE="${1:-creator}"
FRESH="${2:-}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export PATH="/nix/var/nix/profiles/default/bin:$PATH"
command -v nix >/dev/null || { echo "nix not on PATH" >&2; exit 1; }

# The basecamp + lgpm pair a previous `lgs basecamp setup` built. Resolved by
# glob so a re-setup that moves the pin still works.
CACHE="$(ls -d "$HOME"/Library/Caches/logos-scaffold/basecamp/*/ 2>/dev/null | head -1)"
if [ -z "$CACHE" ] || [ ! -x "$CACHE/app-result/bin/LogosBasecamp" ]; then
  echo "No scaffold-built basecamp found under ~/Library/Caches/logos-scaffold/." >&2
  echo "Run 'lgs basecamp setup' once (needs lgs, and about 5 GB free memory)." >&2
  exit 1
fi
BASECAMP="$CACHE/app-result/bin/LogosBasecamp"
LGPM="$CACHE/lgpm-result/bin/lgpm"

# Profile tree. Deliberately NOT under .scaffold/basecamp/profiles: `lgs
# basecamp launch` scrubs that on every invocation, which would wipe the
# creator's identity key and index state (SPEC Q8).
PDIR="$ROOT/.logos/$PROFILE"
USERDIR="$PDIR/Logos/LogosBasecampDev"
mkdir -p "$USERDIR/modules" "$USERDIR/plugins"

echo "==> building module packages"
CORE="$(cd "$ROOT/modules/collect_core" && nix build .#lgx --no-link --print-out-paths | tail -1)"
UI="$(cd "$ROOT/modules/collect_ui" && nix build .#lgx --no-link --print-out-paths | tail -1)"

# Storage and Delivery are NOT in the scaffold-built basecamp. It ships four
# core modules — capability_module, modules_state, package_manager,
# package_downloader — and nothing else. The installed /Applications build does
# have storage_module, but that is the non-dev variant and cannot be mixed with
# ours (see VARIANTS above), so they are built from their own flakes for the
# dev variant and installed like any other module.
#
# COLLECT_NO_NETWORK=1 skips them, for working on the UI without a node.
PLATFORM_MODULES=()
if [ "${COLLECT_NO_NETWORK:-0}" != "1" ]; then
  echo "==> building platform modules (storage, delivery)"
  for flake in logos-storage-module logos-delivery-module; do
    # --accept-flake-config: the Logos binary cache is declared in their
    # flakes as extra-trusted-public-keys, which nix refuses silently for a
    # user who is not in trusted-users. Without the cache these build from
    # source, which takes hours rather than minutes.
    out="$(nix build "github:logos-co/$flake#lgx" --no-link --print-out-paths              --accept-flake-config 2>/dev/null | tail -1)" || out=""
    if [ -n "$out" ]; then
      PLATFORM_MODULES+=("$out")
    else
      echo "    WARNING: $flake did not build; Collect will run without it" >&2
    fi
  done
fi

if [ "$FRESH" = "--fresh" ]; then
  echo "==> wiping installed modules in $PROFILE"
  rm -rf "$USERDIR/modules" "$USERDIR/plugins"
  mkdir -p "$USERDIR/modules" "$USERDIR/plugins"
fi

echo "==> installing"
INSTALL=("$CORE"/*.lgx "$UI"/*.lgx)
for m in "${PLATFORM_MODULES[@]+"${PLATFORM_MODULES[@]}"}"; do
  INSTALL+=("$m"/*.lgx)
done
for f in "${INSTALL[@]}"; do
  "$LGPM" --modules-dir "$USERDIR/modules" --ui-plugins-dir "$USERDIR/plugins" \
          install --file "$f"
done
"$LGPM" --modules-dir "$USERDIR/modules" --ui-plugins-dir "$USERDIR/plugins" list

echo "==> launching basecamp (profile: $PROFILE)"
export LOGOS_USER_DIR="$USERDIR"
export XDG_CONFIG_HOME="$PDIR/xdg-config"
export XDG_DATA_HOME="$PDIR/xdg-data"
export XDG_CACHE_HOME="$PDIR/xdg-cache"
export LOGOS_PROFILE="$PROFILE"
# Short, because liblogos opens a logos_token_<module> socket under TMPDIR and
# macOS caps the whole sockaddr_un path at 104 bytes. An in-profile xdg-tmp
# path overflows it and module loading aborts.
export TMPDIR="/tmp/cc-$PROFILE"
export XDG_RUNTIME_DIR="$TMPDIR"
mkdir -p "$TMPDIR" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"

# SPEC section 6: COLLECT_FIXTURES=1 makes collect_core's read methods answer
# from the embedded example documents. Modules are spawned as child processes
# of basecamp, so they inherit this.
export COLLECT_FIXTURES="${COLLECT_FIXTURES:-1}"

LOG="$PDIR/basecamp.log"
echo "    log: $LOG"
echo "    open Collect from the shell to load collect_ui (which loads collect_core)."
exec "$BASECAMP" > "$LOG" 2>&1
