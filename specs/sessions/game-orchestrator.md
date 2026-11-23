# Session brief: game-orchestrator extension

You are the **Opus planner** for this stream. Plan the work, send each task to a Sonnet worker
(`Agent` with `model: "sonnet"`, independent `[P]` tasks in parallel), review every diff, run the narrowest
check, then commit on your own branch. You are a top-level session: workers cannot start workers.

| | |
| --- | --- |
| Working directory | `/home/jonas/steel/SteelExtensionMC/ee-game-orchestrator/game-orchestrator` |
| Branch | `ext/game-orchestrator` |
| Spec, plan, tasks | `/home/jonas/steel/SteelExtensionMC/ee-game-orchestrator/game-orchestrator/specs/001-game-orchestrator/` |
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
Crate `game-orchestrator/`; the contract crate `game-orchestrator-api/` exists (service `game-orchestrator:games/v1`, action `MatchResult`); you own it. Consumes `arena-manager-api` (`depends: ["arena-manager"]`). Can do now: game types, queue, session open/close, signs, `/queue` and `/orchestrator` commands, handlers for `MatchFinished`/`MatchFailed` re-emitting `MatchResult`. Blocked: kit save/load and the per-player kit in `start_match` need host 007. The sign tasks have a verification step in research R4 first.
