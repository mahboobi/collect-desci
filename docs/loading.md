# Loading the modules into Basecamp

Both modules build. Neither has been **loaded** — no profile exists, and no
call has crossed the real ABI.

```bash
(cd modules/collect_core && nix build .#lgx)   # verified
(cd modules/collect_ui   && nix build .#lgx)   # verified
```

## What blocks loading

`lgs` is gone (it lived in `~/.cargo/bin`). It has no Nix flake, so it has to
be built with cargo from the pinned commit — and that step needs your
approval, because it compiles code from an external repository:

```bash
cargo install --git https://github.com/logos-co/logos-scaffold \
  --rev d4b15ff7ad4e97fd987b9d1216f6e80cb750106e \
  --root "$HOME/.local" --locked
```

`d4b15ff` is the commit `CLAUDE.md` pins. There is no cargo on PATH either, so
prefix with the Nix toolchain:

```bash
export PATH="$(ls -d /nix/store/*-rustc-1.89.0/bin | head -1):$(ls -d /nix/store/*-cargo-1.89.0/bin | head -1):$PATH"
```

(That is rustc 1.89 from the Nix store, not the 1.93 you had. If `lgs` needs a
newer toolchain it will say so, and then rustup is the shorter path.)

## Then, in order

```bash
lgs basecamp setup
```

Pins the basecamp repo, builds `basecamp` + `lgpm`, and seeds the `alice` /
`bob` profile directories. **This is the heavy step**: `CLAUDE.md` says about
5 GB free memory, and the docs add that evaluating the basecamp flake alone
"needs several GB of RAM". This machine has 16 GB with ~4 GB free, so it may
be tight — close what you can first.

```bash
lgs basecamp modules          # or trust the hand-written [modules] table
lgs basecamp install
lgs basecamp launch creator
```

`scaffold.toml` is already written, with both modules and all four SPEC §8
profiles. It deliberately omits the `[repos.*]` pins so `setup` can own them.
If `lgs` rejects the file, move it aside, run `lgs init`, and copy the
`[modules]` and `[basecamp.profiles]` sections across.

## What to check first, once it loads

Start with `collect_core` alone — it is a core module and loads without the
UI. Basecamp's Module Inspector lists its methods and has a **Call** button,
so the first real ABI test needs no UI at all:

| Method | Request | Expect |
| --- | --- | --- |
| `session_capabilities` | — | `{"ok":true,"data":{...,"storage":false,"delivery":false}}` |
| `geo_round` | `{"lat":12.97712,"lon":77.59581}` | `lat 12.975`, `lon 77.595` |
| `experiments_parse_link` | `{"link":"logos-collect://exp/q7k2m3x4w3b5n6tz?m=logos:abc"}` | id, manifest, topic |
| `documents_validate` | `{"document": <examples/submission.json>}` | `valid: true` |

Those four cover the envelope, a pure computation, string handling and the
embedded contract. If `session_capabilities` answers at all, the trait, the
derived `.lidl`, the generated scaffold and the JSON convention are all
working end to end.

Then `collect_ui`, whose backend calls `modules().collect_core.call(...)` — if
its `coreReady` PROP goes true, module-to-module dispatch works.

## Known unknowns that only a running node settles

- **Q9 — RLN.** `logos.test` needs a funded RLN membership before a node
  starts. If a seeded profile does not come with one, the submitter flow in
  SPEC §3.2 cannot work there at all. This is the first thing worth checking
  after a profile launches.
- **Q8 — persistence.** `instance_persistence_path` is the right directory,
  but `lgs basecamp launch` scrubs the profile tree on every invocation by
  design. For the §8 test that restarts the creator, hand-launch Basecamp with
  `--user-dir` pointing outside `.scaffold/basecamp/profiles/`.
- Whether `.#lgx`'s `darwin-arm64-dev` variant is what `lgpm` wants to install,
  or whether a release variant is needed.
