//! Fluid behavior registry.

use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use steel_registry::fluid::{FluidRef, FluidState};
use steel_registry::{REGISTRY, RegistryEntry, RegistryExt};
use steel_utils::locks::SyncMutex;
use steel_utils::{BlockPos, Identifier};

use crate::fluid::{EmptyFluid, ExtensionFluid, FluidBehavior, FluidHooks};
use crate::world::World;

/// Decides whether a fluid may spread at a position, before vanilla's own checks.
/// Returning `false` stops it, so a listener that does must also change the block.
pub type FluidFlowListener = Box<dyn Fn(&Arc<World>, BlockPos, FluidState) -> bool + Send + Sync>;

type RegistrationHook = Box<dyn FnOnce(&mut FluidBehaviorRegistry) + Send>;
static REGISTRATION_HOOKS: SyncMutex<Option<Vec<RegistrationHook>>> =
    SyncMutex::new(Some(Vec::new()));

/// Wrapper for the global fluid behavior registry that implements `Deref`.
pub struct FluidBehaviorLock(pub OnceLock<FluidBehaviorRegistry>);

impl Deref for FluidBehaviorLock {
    type Target = FluidBehaviorRegistry;

    fn deref(&self) -> &Self::Target {
        self.0.get().expect("Fluid behaviors not initialized")
    }
}

/// Global fluid behavior registry.
///
/// Access behaviors directly via deref: `FLUID_BEHAVIORS.get_behavior(fluid)`
pub static FLUID_BEHAVIORS: FluidBehaviorLock = FluidBehaviorLock(OnceLock::new());

/// Registry for fluid behaviors.
///
/// Created after the main registry is frozen. All fluids are initialized with
/// default behavior ([`EmptyFluid`]), then custom behaviors are registered.
pub struct FluidBehaviorRegistry {
    behaviors: Vec<Box<dyn FluidBehavior>>,
    /// Source fluid ids whose owner already attached hooks.
    hooked: Vec<usize>,
    flow_listeners: Vec<FluidFlowListener>,
}

impl FluidBehaviorRegistry {
    /// Creates a new behavior registry with default behaviors for all fluids.
    #[must_use]
    pub fn new() -> Self {
        let fluid_count = REGISTRY.fluids.len();
        let mut behaviors: Vec<Box<dyn FluidBehavior>> = Vec::with_capacity(fluid_count);

        // Initialize all fluids with default behavior (EmptyFluid)
        for _ in 0..fluid_count {
            behaviors.push(Box::new(EmptyFluid));
        }

        Self {
            behaviors,
            hooked: Vec::new(),
            flow_listeners: Vec::new(),
        }
    }

    /// Sets a custom behavior for a fluid.
    ///
    /// # Panics
    /// Panics if `fluid` is not registered in the global registry.
    pub fn set_behavior(&mut self, fluid: FluidRef, behavior: Box<dyn FluidBehavior>) {
        let id = fluid.id();
        self.behaviors[id] = behavior;
    }

    /// Installs behavior edits before the registry is published. Subscribe
    /// before behaviors initialize, like block behavior registration.
    ///
    /// # Errors
    /// Returns an error if fluid behaviors were already initialized.
    pub fn on_registration(hook: impl FnOnce(&mut Self) + Send + 'static) -> Result<(), String> {
        let mut hooks = REGISTRATION_HOOKS.lock();
        let Some(hooks) = hooks.as_mut() else {
            return Err("fluid behavior registration has already finished".into());
        };
        hooks.push(Box::new(hook));
        Ok(())
    }

    pub(crate) fn run_registration_hooks(&mut self) {
        let hooks = REGISTRATION_HOOKS.lock().take();
        if let Some(hooks) = hooks {
            for hook in hooks {
                hook(self);
            }
        }
    }

    /// Gives every registered fluid its host-run behavior, without hooks.
    pub(crate) fn register_client_fluids(&mut self) {
        for &fluid in REGISTRY.client_content.fluids() {
            self.set_behavior(fluid.source, Box::new(ExtensionFluid::new(fluid, None)));
            self.set_behavior(fluid.flowing, Box::new(ExtensionFluid::new(fluid, None)));
        }
    }

    /// Adds a flow listener, called for every fluid in registration order.
    pub fn add_flow_listener(&mut self, listener: FluidFlowListener) {
        self.flow_listeners.push(listener);
    }

    /// Asks the flow listeners whether the fluid at `pos` may spread. Stops at the
    /// first refusal.
    ///
    /// Vanilla has no such step. Called at the head of `LiquidBlock::should_spread_liquid`,
    /// so vanilla's lava rules stay in place and always run when no listener refuses.
    #[must_use]
    pub fn allow_flow(&self, world: &Arc<World>, pos: BlockPos, fluid: FluidState) -> bool {
        self.flow_listeners
            .iter()
            .all(|listener| listener(world, pos, fluid))
    }

    /// Attaches gameplay hooks to a registered fluid, given by its source key.
    /// Hooks can be attached once, so a second extension cannot replace them.
    ///
    /// # Errors
    /// Returns an error if the key does not name a registered fluid or the fluid
    /// already has hooks.
    pub fn set_fluid_hooks(
        &mut self,
        source: &Identifier,
        hooks: Arc<dyn FluidHooks>,
    ) -> Result<(), String> {
        let fluid = REGISTRY
            .fluids
            .by_key(source)
            .and_then(|fluid| REGISTRY.client_content.fluid(fluid))
            .ok_or_else(|| format!("{source} is not a registered extension fluid"))?;
        let id = fluid.source.id();
        if self.hooked.contains(&id) {
            return Err(format!("{source} already has fluid hooks"));
        }
        self.hooked.push(id);
        let reacting = Arc::clone(&hooks);
        self.add_flow_listener(Box::new(move |world, pos, state| {
            let own_fluid = std::ptr::eq(state.fluid_id, fluid.source)
                || std::ptr::eq(state.fluid_id, fluid.flowing);
            !own_fluid || reacting.react_with_neighbors(world, pos, world.get_block_state(pos))
        }));
        self.set_behavior(
            fluid.source,
            Box::new(ExtensionFluid::new(fluid, Some(Arc::clone(&hooks)))),
        );
        self.set_behavior(
            fluid.flowing,
            Box::new(ExtensionFluid::new(fluid, Some(hooks))),
        );
        Ok(())
    }

    /// Gets the behavior for a fluid.
    ///
    /// # Panics
    /// Panics if `fluid` is not registered in the global registry.
    #[must_use]
    pub fn get_behavior(&self, fluid: FluidRef) -> &dyn FluidBehavior {
        let id = fluid.id();
        self.behaviors[id].as_ref()
    }
}

impl Default for FluidBehaviorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use steel_registry::{init_vanilla_registry, vanilla_fluids};

    use super::*;
    use crate::test_support::fresh_test_world;

    fn counting(calls: &Arc<AtomicU32>, allow: bool) -> FluidFlowListener {
        let calls = Arc::clone(calls);
        Box::new(move |_, _, _| {
            calls.fetch_add(1, Ordering::Relaxed);
            allow
        })
    }

    #[test]
    fn flow_is_allowed_until_a_listener_refuses() {
        init_vanilla_registry();
        let fixture = fresh_test_world("fluid_flow_listeners");
        let water = FluidState::source(&vanilla_fluids::WATER);
        let pos = BlockPos::new(0, 64, 0);
        let mut registry = FluidBehaviorRegistry::new();
        assert!(
            registry.allow_flow(&fixture.world, pos, water),
            "no listener, no effect"
        );

        let (first, refusing, last) = (
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU32::new(0)),
        );
        registry.add_flow_listener(counting(&first, true));
        registry.add_flow_listener(counting(&refusing, false));
        registry.add_flow_listener(counting(&last, true));
        assert!(!registry.allow_flow(&fixture.world, pos, water));
        assert_eq!(first.load(Ordering::Relaxed), 1);
        assert_eq!(refusing.load(Ordering::Relaxed), 1);
        assert_eq!(
            last.load(Ordering::Relaxed),
            0,
            "listeners stop at the first refusal"
        );
    }
}
