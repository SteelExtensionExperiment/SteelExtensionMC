# Parallel sessions

One Opus 5.5 (high effort) session per stream, each in its own git worktree inside this repo directory.
Start a session with `cd <working directory> && claude --model opus`, set effort high, and tell it:
`Read /home/jonas/steel/SteelExtensionMC/specs/sessions/<stream>.md and follow it.`

| Stream | Worktree | Branch | Brief |
| --- | --- | --- | --- |
| host-005 | `host-005` | `host/005-world-templates` | [host-005.md](host-005.md) |
| host-006 | `host-006` | `host/006-text-holograms` | [host-006.md](host-006.md) |
| host-007 | `host-007` | `host/007-item-snapshot` | [host-007.md](host-007.md) |
| world-cloner | `/home/jonas/steel/SteelExtensionMC/ee-world-cloner/world-cloner` | `ext/world-cloner` | [world-cloner.md](world-cloner.md) |
| arena-manager | `/home/jonas/steel/SteelExtensionMC/ee-arena-manager/arena-manager` | `ext/arena-manager` | [arena-manager.md](arena-manager.md) |
| game-orchestrator | `/home/jonas/steel/SteelExtensionMC/ee-game-orchestrator/game-orchestrator` | `ext/game-orchestrator` | [game-orchestrator.md](game-orchestrator.md) |
| stats-visualizer | `/home/jonas/steel/SteelExtensionMC/ee-stats-visualizer/stats-visualizer` | `ext/stats-visualizer` | [stats-visualizer.md](stats-visualizer.md) |

The coordinator (the session that created these) merges `host/*` into the main checkout and `ext/*` into
`extension-example`. Extensions build against the main checkout's `steel-extension-api`, so a host
milestone is visible to them only after it is merged.
