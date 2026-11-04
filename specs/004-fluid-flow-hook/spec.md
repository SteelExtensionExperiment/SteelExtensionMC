# Feature Specification: Fluid flow listeners

**Feature Branch**: `004-fluid-flow-hook`

**Created**: 2026-11-03

**Status**: Implemented

**Input**: Refactor iteration. Iteration 003 moved lava's reactions out of `LiquidBlock` into `LavaFluid` behind `FluidBehavior::react_with_neighbors`. No reference project does that. Put vanilla's code back and use the lightest extension point instead.

## Reference survey (all on 26.3)

| Project | Where lava's rules live | Extension point |
| --- | --- | --- |
| Vanilla | `LiquidBlock.shouldSpreadLiquid` | None |
| Fabric API | `LiquidBlock`, unchanged | `FluidFlowEvents.ALLOW`, injected at the head of `shouldSpreadLiquid`, plus `spreadTo` |
| NeoForge | Table of rules | `FluidInteractionRegistry`, replacing the call in `neighborChanged` only |
| Paper | `LiquidBlock` | Bukkit events around the conversions |

Fabric's design is the closest to vanilla and the smallest change, so this iteration follows it.

## User Scenarios & Testing

### User Story 1 - Vanilla code reads like vanilla (Priority: P1)

A maintainer compares `LiquidBlock::should_spread_liquid` with `LiquidBlock.java` and finds the same rules in the same place.

**Acceptance Scenarios**:

1. **Given** the refactored method, **When** compared with the version before iteration 003, **Then** the only difference is the flow listener step at its head.
2. **Given** no flow listener is registered, **When** lava meets water, or blue ice over soul soil, **Then** the result is the vanilla one.

### User Story 2 - Fluids react before spreading (Priority: P1)

A registered fluid reacts to its neighbors at the moment vanilla decides whether a liquid spreads, on placement and on neighbor changes.

**Acceptance Scenarios**:

1. **Given** a sludge block placed next to lava, **When** the placement runs, **Then** the lava cools into coarse dirt (source) or cobblestone (flowing).
2. **Given** several listeners, **When** one refuses, **Then** the later ones do not run and the liquid does not spread.

## Requirements

- **FR-001**: Lava's obsidian, cobblestone, and basalt rules MUST return to `LiquidBlock::should_spread_liquid`, unchanged.
- **FR-002**: `FluidBehaviorRegistry` MUST offer flow listeners, called in registration order at the head of `should_spread_liquid`, stopping at the first refusal.
- **FR-003**: `FluidBehavior::react_with_neighbors` MUST be removed.
- **FR-004**: The extension API and the toxic-wasteland extension MUST NOT change: `FluidHooks::react_with_neighbors` runs through a flow listener that filters its own fluid.
- **FR-005**: The deviation from vanilla MUST be documented on the method.

## Out of Scope

- A listener on each flow step (`spreadTo`), which Fabric also has. Nothing needs it yet.
- Exposing global flow listeners to extensions. They are a host API for now, and extensions keep their per-fluid hook.

## Success Criteria

- **SC-001**: The core and all extension API suites pass, with existing lava tests unchanged.
- **SC-002**: The sludge's lava reaction is tested through real block placement.
