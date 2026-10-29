#!/usr/bin/env bash
# Run cargo for the logic crate without rustup.
#
# ~/.cargo and ~/.rustup vanished mid-development (see docs/api-verification.md);
# a rustup shim is back on PATH but has no default toolchain, so a bare `cargo`
# fails with "could not choose a version of cargo to run". These two store paths
# are the toolchain the Nix module build itself uses, so testing against them
# keeps `cargo test` and `nix build .#lgx` on one compiler.
set -euo pipefail
CARGO=/nix/store/gzv6psv17kv7x9s01w8jhi0h2cg6z15p-cargo-1.89.0/bin/cargo
RUSTC=/nix/store/j1z0svvgbi5hnl583lz45iqjj60nswm1-rustc-1.89.0/bin
[ -x "$CARGO" ] || { echo "no pinned cargo at $CARGO" >&2; exit 1; }
export PATH="$RUSTC:$PATH"
export RUSTUP_TOOLCHAIN=
exec "$CARGO" "$@"
