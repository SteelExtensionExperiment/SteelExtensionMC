# Implementation Plan: Fluid flow listeners

**Branch**: `004-fluid-flow-hook` | **Date**: 2026-11-03 | **Spec**: [spec.md](spec.md)

## Summary

Restore vanilla's `should_spread_liquid`, and add one step at its head that asks flow listeners held by `FluidBehaviorRegistry`, like Fabric's `FluidFlowEvents.ALLOW`. Fluid hooks attach as a listener filtered to their own fluid, so nothing changes above the core.

## Constitution Check

| Principle | Status | How |
| --- | --- | --- |
| I. Stable Native Boundary | Pass | No binding changes. |
| II. Modding-Grade Extensibility | Pass | Listeners see every fluid, so more than one owner can react to lava. |
| III. Vanilla Parity First | Pass | Vanilla's code is back in place. With no listener, `allow_flow` returns `true`. The deviation is documented on the method. |
| IV. Foundation Rigor | Pass | `spreadTo` coverage is explicitly out of scope. |
| VII. Verified Behavior | Pass | Unit test for listener order and refusal; native test through real placement. |

## Source Code

```text
steel-core/src/behavior/fluid.rs                      # FluidFlowListener, add_flow_listener, allow_flow
steel-core/src/behavior/blocks/fluid/liquid_block.rs  # vanilla rules restored, listener step added
steel-core/src/fluid/fluid_behavior.rs                # react_with_neighbors removed
steel-core/src/fluid/fluids/{lava,extension}.rs       # react_with_neighbors implementations removed
steel-extension-api/tests/toxic_wasteland.rs          # lava reaction through real placement
```
