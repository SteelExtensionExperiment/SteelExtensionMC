# Session brief: world-cloner extension

You are the **Opus planner** for this stream. Plan the work, send each task to a Sonnet worker
(`Agent` with `model: "sonnet"`, independent `[P]` tasks in parallel), review every diff, run the narrowest
check, then commit on your own branch. You are a top-level session: workers cannot start workers.

| | |
| --- | --- |
| Working directory | `/home/jonas/steel/SteelExtensionMC/ee-world-cloner/world-cloner` |
| Branch | `ext/world-cloner` |
| Spec, plan, tasks | `/home/jonas/steel/SteelExtensionMC/ee-world-cloner/world-cloner/specs/001-world-cloner/` |
| Vanilla source (target `0.17.1+mc26.3`) | `/home/jonas/minecraft/minecraft_26.3/net/minecraft/` (`version.json` id `26.3`); the repo's `minecraft-src/` is not generated |
| Builds | `export CARGO_BUILD_JOBS=2` (12 cores, 30 GB shared by 7 sessions) |

## Read first
1. `/home/jonas/steel/SteelExtensionMC/AGENTS.md` (applies in full) and `/home/jonas/steel/SteelExtensionMC/specs/extensions-workflow.md`.
2. `/home/jonas/steel/SteelExtensionMC/specs/extension-communication.md`: services go down, hook actions come up. Emit actions after releasing every lock.
3. Your own spec, plan, tasks and `contracts/`.

## Rules
- Work only in your directory. Never edit another stream, the shared example index, `package-extension.py` or the root `README.md`.
- Commit freely on your branch. Do not push. Do not merge; the coordinator merges.
- Missing host capability, vanilla reference, extractor data or decision: **stop and ask the user** with the AGENTS.md template. No stubs, no hardcoded vanilla data, no guessing.
- A published contract changes only with the user's approval.
- Report to the user when a milestone lands (what a dependent stream can now use).

## This stream
Crates: `world-cloner/` and a new sibling `world-cloner-api/` (same layout and lints as `vault-api`). Can do now: crate scaffold, `config.toml` parsing and validation, `/worldcloner` command tree with permission nodes, tests for config. Blocked until host 005 is merged into the main checkout: everything that calls the host world bindings, and `world-cloner-api` (it uses `Operation` and `WorldSettings`). Record the blocker in `plan.md` and stop at it.
