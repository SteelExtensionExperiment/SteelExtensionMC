# Tasks: Fluid flow listeners

- [x] T001 Move lava's reactions from `steel-core/src/fluid/fluids/lava.rs` back into `LiquidBlock::should_spread_liquid`.
- [x] T002 Add `FluidFlowListener`, `add_flow_listener`, and `allow_flow` to `steel-core/src/behavior/fluid.rs`, with a unit test for order and refusal.
- [x] T003 Call `allow_flow` at the head of `should_spread_liquid`, and document the deviation.
- [x] T004 Remove `FluidBehavior::react_with_neighbors`. `set_fluid_hooks` adds a listener filtered to its fluid.
- [x] T005 Test the sludge's lava reaction through real placement in `steel-extension-api/tests/toxic_wasteland.rs`.
- [x] T006 Run the core and API suites, clippy, and rustfmt.

Result: `should_spread_liquid` differs from its pre-003 version only by the listener step. The core suite (2973 tests) and all 17 API suites pass.

Seen while testing: `creatures_and_magic` (added on master) timed out once in a full parallel run and passed in the next one. It passes alone on this branch and on master, so it looks load-sensitive and unrelated to this change.
