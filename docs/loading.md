# Loading Collect into Basecamp

Both modules build, install, and are accepted by Basecamp. Basecamp is
launchable against a local profile with one command:

```bash
tools/load.sh            # profile "creator"
tools/load.sh alice      # any profile
tools/load.sh creator --fresh
```

Then **open Collect from the Basecamp shell** — that is what loads
`collect_ui`, which loads `collect_core`. Modules are loaded on demand, not at
startup.

## `lgs` is not needed

A previous `lgs basecamp setup` left everything in the Nix store, and
`tools/load.sh` uses it directly:

| | |
| --- | --- |
| Basecamp | `~/Library/Caches/logos-scaffold/basecamp/<rev>/app-result/bin/LogosBasecamp` |
| `lgpm` | `.../lgpm-result/bin/lgpm` (`pre-release-d3af297`) |

`lgs` itself is gone (it lived in `~/.cargo/bin`). Rebuilding it needs
`cargo install --git`, which this environment refuses as external code — but
nothing here depends on it any more.

## The variant trap

This cost the most time, and it bites from both directions.

| Attribute | Variant it produces |
| --- | --- |
| `.#lgx` | `darwin-arm64-dev` |
| `.#lgx-portable` | `darwin-arm64` |

The scaffold-pinned basecamp (`packages.app`, run as `bin/LogosBasecamp`) and
the pinned `lgpm` both speak **`darwin-arm64-dev`**, so `.#lgx` is the right
one here.

The `0.3.0` tag's `bin-macos-app` is the opposite — it accepts only
`[darwin-arm64, darwin-aarch64]`:

> module 'collect_core' ... was installed for variant 'darwin-arm64-dev'
> which is not supported on this platform and will not be loadable

and feeding it the portable build instead just moves the failure to install
time, because `lgpm` then refuses the package it cannot run:

> Package does not contain variant for platform: darwin-arm64-dev
> (package provides: darwin-arm64)

`lgpm --platform darwin-arm64` does **not** override this in the dev build. The
only combination that works is the pinned pair plus `.#lgx`. This is the
`lgpm` ↔ `bin-macos-app` alignment problem `lgs basecamp docs` alludes to.

The confirming evidence: the Open Labbook project in `~/development/logos` has
`olb_core`, `olb_ui`, `storage_module`, `delivery_module` and the two RLN
modules all installed as `darwin-arm64-dev`, and they load.

## Verified

- `lgpm install` accepts both packages — content hashes validate, only an
  "unsigned" warning. Dependency graph resolves: `deps collect_ui` →
  `collect_core`.
- Basecamp launches against the profile, reports the right base data
  directory, and raises **no** variant or validation complaint about either
  module.
- The plugin exports the whole module C ABI: `logos_module_install`,
  `_dispatch`, `_get_methods`, `_get_protocol_version`, `_accept_token`,
  `_accept_inbound_token`, `_grant_host_services`, `_about_to_unload`. arm64,
  with 63 of our own Rust symbols in it.

## Not verified

**No method has been called across the real ABI.** Loading is on demand and
needs a click in the shell, which this environment cannot do — computer-use is
not set up here.

Running the module standalone does not work around that, and should not:

```
logos_host --name collect_core --path .../collect_core_plugin.dylib ...
  [critical] No auth token received from source: stdin
  @logos-load-status failed no auth token arrived on stdin
```

Every module needs a token issued by `capability_module`'s registry. That is
the security model doing its job, and forging one is not a test.

## What to try once Collect is open

`collect_core` has 13 methods; four of them cover the whole stack, and the
Module Inspector has a **Call** button:

| Method | Request | Expect |
| --- | --- | --- |
| `session_capabilities` | — | `{"ok":true,...,"storage":false,"delivery":false}` |
| `geo_round` | `{"lat":12.97712,"lon":77.59581}` | `lat 12.975`, `lon 77.595` |
| `experiments_parse_link` | `{"link":"logos-collect://exp/q7k2m3x4w3b5n6tz?m=logos:abc"}` | id, manifest, topic |
| `documents_validate` | `{"document": <examples/submission.json>}` | `valid: true` |

If `session_capabilities` answers at all, the trait, the derived `.lidl`, the
generated scaffold and the JSON envelope all work end to end.

For the UI: `collect_ui`'s `coreReady` PROP going true means module-to-module
dispatch through `modules().collect_core` works.

## Two things worth fixing on this machine

**You are not a trusted Nix user.** Two consequences, both silent:

- Your `~/.config/nix/nix.conf` download-timeout tuning is ignored
  (`ignoring the client-specified setting 'stalled-download-timeout'`), so the
  ISP workaround you wrote is not in effect for daemon builds.
- Basecamp's flake declares `extra-substituters` and
  `extra-trusted-public-keys` — its **binary cache** — and Nix ignores those
  too. Anything not already in the store gets built from source rather than
  downloaded.

Either add yourself to `trusted-users` in `/etc/nix/nix.conf`, or move those
settings there.

**`~/.zshenv` sources `~/.cargo/env`, which no longer exists**, so every shell
prints an error. Remove the line, or reinstall rustup.

## Still open

- **Q9 — RLN.** `logos.test` needs a funded RLN membership before a node
  starts. The Open Labbook profile has `liblogos_rln_module` and
  `liblogos_lez_rln_module` installed, which suggests the pieces exist on this
  machine — worth checking whether that profile has a funded membership before
  assuming Collect's submitters cannot announce.
- **Q12** — whether `collect_ui` keeps its C++ backend. Now that the `.lidl`
  exists, the `ui-qml` route would let QML call the 13 methods with no C++ in
  between.
