# Session brief: arena-manager extension

You are the **Opus planner** for this stream. Plan the work, send each task to a Sonnet worker
(`Agent` with `model: "sonnet"`, independent `[P]` tasks in parallel), review every diff, run the narrowest
check, then commit on your own branch. You are a top-level session: workers cannot start workers.

| | |
| --- | --- |
| Working directory | `/home/jonas/steel/SteelExtensionMC/ee-arena-manager/arena-manager` |
| Branch | `ext/arena-manager` |
| Spec, plan, tasks | `/home/jonas/steel/SteelExtensionMC/ee-arena-manager/arena-manager/specs/001-arena-manager/` |
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
Crate `arena-manager/`; the contract crate `arena-manager-api/` already exists next to it (service `arena-manager:match/v1`, actions `MatchFinished`/`MatchFailed`); you own it. Can do now: `arenas.toml` and `config.toml` storage and validation, `/arena` commands, match state machine, emitting the actions. Blocked: world cloning needs `world-cloner-api` (after host 005 and the world-cloner stream); `PlayerEntry.kit` needs host 007. Do not copy world-cloner types. `depends: ["world-cloner"]`.
