# Implementation Plan: Extension-controlled Text Holograms (host platform)

**Branch**: `006-text-holograms` | **Date**: 2026-11-23 | **Spec**: [spec.md](spec.md)

## Summary

Expose a small, lifecycle-managed hologram capability to extensions: create, set text, remove. The host
owns a registry of holograms, backed by the existing `TextDisplayEntity`, never persisted, recreated on
chunk load and dropped with its world. It exists so the `stats-visualizer` extension can show a floating
board; it changes no existing binding.

## Technical Context

**Language/Version**: Rust nightly (`rust-toolchain.toml`).

**Primary Dependencies**: none new. Uses `TextDisplayEntity`, the entity tracker, and the extension
runtime bindings.

**Storage**: none (H2).

**Testing**: host tests beside the code plus a native probe test (T006), as in spec 005.

**Constraints**: no async, no threads; chunk-lifecycle work stays on the existing entity-manager paths
(research R4); registry behind one `SyncMutex` (`steel_utils::locks`).

**Scale/Scope**: tens of holograms per server.

## Constitution Check

| Principle | Status | How |
| --- | --- | --- |
| I. Stable Native Boundary | Pass | New types are stabby with a plain `u64` handle; no host pointers cross. |
| II. Modding-Grade Extensibility | Pass | Opaque non-reused handle; no vanilla-specific key leaks. |
| III. Vanilla Parity First | Pass | A1 verified against vanilla 26.3 (research R1). |
| IV. Foundation Rigor | Pass | Whole lifecycle (create, update, unload, load, drop) is in scope; no stubs. |
| V. Explicit Failure | Pass | Every failure is a message. |
| VI. Concurrency & Dependencies | Pass | One mutex, no threads, no new crates. |
| VII. Verified Behavior | Pass | Unit tests and a native probe test. |

## Project Structure

```text
steel-core/src/
├── entity/entities/objects/technical/display/text_display.rs   # CHANGED: non-persistence (T002)
└── world/hologram.rs                                            # NEW: HologramRegistry
steel-core/src/world/entity_management.rs                        # CHANGED: chunk load/unload calls
steel-extension-api/src/
├── runtime/types.rs        # CHANGED: HologramId, Billboard
├── runtime/bindings.rs     # CHANGED: three Server methods
└── host/runtime.rs         # CHANGED: adapters
steel-extension-api/tests/hologram-probe/                        # NEW probe extension
steel-extension-api/tests/holograms.rs                           # NEW native test
doc/extensions/api.md                                            # CHANGED: document the bindings
```

## Complexity Tracking

Empty.
