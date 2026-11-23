# Quickstart: Extension-controlled Text Holograms (host platform)

Run from the Steel checkout.

## Automated checks

```sh
cargo test -p steel-core hologram
cargo test -p steel-extension-api --test holograms -- --ignored   # after building the probe
cargo clippy -r --workspace --all-targets --all-features
cargo fmt --all --check
typos
```

Build the probe first (`tests/hologram-probe/`), as spec 005's probe.

## Manual check in a real client

1. Start Steel with the probe extension and `/op` yourself.
2. Run the probe's create command at your position. A text appears, turned toward you, wherever you walk
   around it.
3. Run its set-text command. The text changes without flicker.
4. Move far away until the chunk unloads, return: the text is there once.
5. `/save-all`, stop, inspect the world's region files for a text display: none.
6. Start again without the probe recreating it: no text. Recreate it: exactly one.
7. Remove it: it disappears. Unload or remove its world: it is gone and its handle reports an error.
8. `/summon minecraft:text_display ~ ~ ~ {text:'"x"'}` still creates a normal saved text display.
