#!/usr/bin/env bash
# Everything that can be checked without a node.
#
#   tools/test.sh
#
# Four suites, because they cover different things and none subsumes another:
# the logic crate, the two platform clients (which cargo can only reach
# through tools/clients-check), the QML, and the data contract.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="/nix/var/nix/profiles/default/bin:$PATH"

echo "==> collect_core_logic"
"$ROOT/tools/cargo.sh" test --quiet \
  --manifest-path "$ROOT/modules/collect_core/rust-lib/core/Cargo.toml"

echo "==> storage and delivery clients"
"$ROOT/tools/cargo.sh" test --quiet \
  --manifest-path "$ROOT/tools/clients-check/Cargo.toml"

echo "==> collect_ui (QML)"
bash "$ROOT/modules/collect_ui/tests/run.sh" | tail -2

echo "==> the data contract"
# The system python has no jsonschema, so it comes from nix.
nix-shell -p 'python3.withPackages (ps: [ ps.jsonschema ])' \
  --run "python3 '$ROOT/tools/validate.py'" | tail -1
