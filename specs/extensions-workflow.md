# Parallel extension workflow

Four new extensions are planned together, then built in parallel by separate agents, **one directory
each** inside `extension-example/`. This file is the ownership map. Agents must read it and `AGENTS.md` before touching anything.

## Repositories

Each extension lives in `extension-example/`, the examples repository checked out here as a submodule,
next to the existing examples. Changes to them are committed in that repository, not in this one.

| Extension | Local path | Spec | Status |
| --- | --- | --- | --- |
| World cloner | `extension-example/world-cloner/` | `extension-example/world-cloner/specs/001-world-cloner/` | spec, plan and tasks drafted; open items in its checklist |
| Arena manager | `extension-example/arena-manager/` | `extension-example/arena-manager/specs/001-arena-manager/` | not started |
| Stats visualizer | `extension-example/stats-visualizer/` | `extension-example/stats-visualizer/specs/001-stats-visualizer/` | spec, plan and tasks drafted; open items in its checklist |
| Game orchestrator | `extension-example/game-orchestrator/` | `extension-example/game-orchestrator/specs/001-game-orchestrator/` | not started |

Host platform work lives in this checkout's `specs/`:

| Spec | Purpose | Status |
| --- | --- | --- |
| `005-world-templates` | Read-only templates, copy-on-write RAM worlds, runtime world creation and removal, and the extension API bindings the world cloner needs | spec, plan and tasks drafted; open items in its checklist |
| `006-text-holograms` | Extension-controlled floating text (create, update, remove), never persisted, kept across chunk unload, dropped with its world | spec, plan and tasks drafted; vanilla source missing (`minecraft-src/`), decisions marked confirm |

Number 004 is taken upstream by `004-fluid-flow-hook`, so new host specs continue from 005 and 006 is taken; the next free number is 007.

## Layout

```text
SteelExtensionMC/                 # Steel checkout (this repository)
├── steel-extension-api/
├── specs/                        # host platform specs and this file
└── extension-example/            # submodule: examples and the four planned extensions
    ├── world-cloner/             # crate and specs/001-world-cloner/
    ├── world-cloner-api/         # contract crate, created by the spec's tasks
    ├── arena-manager/            # crate and specs/001-arena-manager/
    ├── arena-manager-api/
    ├── game-orchestrator/        # crate and specs/001-game-orchestrator/
    ├── game-orchestrator-api/
    └── stats-visualizer/         # crate and specs/001-stats-visualizer/ (no api crate)
```

Same layout as `vault` and `vault-api`: the crate directory holds `Cargo.toml`, `src/`, `README.md` and
its `specs/`; the contract crate is a sibling. A crate sits two levels below this checkout, so
`../../steel-extension-api` resolves. Another extension depends on a contract crate by sibling path, for
example `world-cloner-api = { path = "../world-cloner-api" }`.

Extensions use the checkout's `rust-toolchain.toml`, which Cargo finds from there. Each crate has its own
`[workspace]`, as the examples do.

`package-extension.py` takes any crate directory: `python3 package-extension.py extension-example/world-cloner`.

## Ownership rules

- An agent works only inside its own extension directories (`<name>/` and `<name>-api/`). It does not edit another extension.
- **Host changes** (`steel-extension-api`, `steel-core`, and so on) are shared by every extension. An
  agent that finds a missing platform capability records it as a platform gap in its own `plan.md` and
  stops. The user decides who builds it and in what order, as in spec 003. Host work is specified in this
  checkout's `specs/`.
- **Commits and pushes:** agents do not commit or push. The user does, and creates any remotes.
- **Blockers:** `AGENTS.md` applies in full. Missing vanilla reference, API, or decision means stop and
  ask; do not stub or guess.
- The shared example index `extension-example/README.MD`, the other examples, `package-extension.py` and
  the root `README.md` are not edited by extension work.

## Cross-extension contracts

- The extension that provides a service owns the contract. It documents it in its own
  `specs/<spec>/contracts/` and puts the types in a sibling `<name>-api` crate, like
  `vault-api`, `worldedit-api` and `prison-api`.
- A consumer links to the provider's contract and never copies it. Its `depends` or `soft_depends` entry
  names the provider. Dependencies are declared in the long `export_extension!` form, not in
  `extension.toml`, which accepts only `[package]` and `[targets]`
  (`steel-extension-api/src/lib.rs:221-264`, `steel-extension-api/src/host/package.rs:15-26`).
- A published contract changes only with the user's approval, because other extensions build against
  it.

## Native tests

The shared native-test helper resolves libraries at `../extension-example/<crate>/target/debug/`
(`steel-extension-api/tests/common/mod.rs:222-233`), with one special case for the in-tree
`tests/event-probe`. The new extensions now sit at that path, so the helper can find them. It is a test
module, not a library, so it is still not importable from outside `steel-extension-api`.
Host behavior is therefore tested in this checkout with probe extensions (spec 005, T001 and T019). Where
an extension's own native test lives is an open point per extension (world cloner: O1).

## Dependency graph and build order

Set after all four specs list their provided and consumed contracts. Known so far:

- **World cloner** provides `world-cloner:clone/v1` and depends on host spec 005. The arena manager is
  planned to consume it.
- **Extension implementations** that use the new host bindings wait for the matching tasks in spec 005.
  The world cloner's own task list names which. Specs and plans for the other extensions can proceed now,
  against `extension-example/world-cloner/specs/001-world-cloner/contracts/service.md`.
- **Stats visualizer** consumes the `game-orchestrator` contract (game types and match results) and depends on host spec 006 (holograms). Its contract needs are in `extension-example/stats-visualizer/specs/001-stats-visualizer/contracts/required-orchestrator.md`, a proposal until the orchestrator spec adopts it. Its tasks T001-T005 need neither.
- Host changes from any spec are serialized by the user.
