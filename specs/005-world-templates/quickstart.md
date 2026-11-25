# Quickstart: Runtime RAM Worlds from Saved Templates (host platform)

Run from the Steel checkout (`SteelExtensionMC/`). The platform is checked with host tests and a probe
extension kept in this repository; no extension is needed.

## Prerequisites

- The tasks of the phase you are checking in [tasks.md](tasks.md) are done.
- For manual checks, a saved Steel world to use as a template: any world folder written by this Steel
  version, containing `region/` and `level.toml`. For a quick one, start a server with an empty
  `steel:empty` world, build something, stop it, and copy `saves/<domain>/worlds/<name>/` somewhere.

## Automated checks

```sh
# Host unit tests
cargo test -p steel-core -- chunk_saver world::template server::templates server::temporary
cargo test -p steel-extension-api

# Native integration (builds the probe extension in debug mode first)
cargo build --manifest-path steel-extension-api/tests/world-templates-probe/Cargo.toml
cargo test --profile cicd -p steel-extension-api --features host --test world_templates -- --include-ignored

# Project checks
cargo clippy -r --workspace --all-targets --all-features
cargo fmt --all --check
typos
```

## Manual check with the probe extension

1. Copy `steel-extension-api/tests/world-templates-probe`'s built library into `extensions/` and start
   Steel. `/op` yourself from the console.
2. Load a template by its probe command; the template folder is not modified, and its status turns Ready.
3. Clone it as `game_1:map` into a new domain, enter it with `/domain game_1:map`, and check blocks
   and chests. Animals and armor stands from the original are not there.
4. Clone it as `game_2:map`, enter it, break a block in `game_1:map`, and check `game_2:map` is
   unchanged. Day and night advance independently in both.
5. With a lasting domain configured, clone into it, restart the server, and check the clone is gone
   while the domain's data is intact.
6. Stand in `game_1:map`, remove it with the probe command: you are moved to the domain's default world,
   or the server default world when there is none. Removing a world from `worlds.toml` is refused.
7. Compare the template folder with the original copy: identical files and modification times.

## Failure checks

- A template path that does not exist, or a folder written by a different Steel format version: the
  template shows Failed with a message, other templates still clone, and the folder is untouched (no
  `*.bak` file appears).
