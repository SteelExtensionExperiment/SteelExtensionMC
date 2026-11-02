//! Temporary AI decision controllers. Native sensing, controls and physics still tick.
//! Without an active controller, mobs run their ordinary vanilla AI.

use crate::entity::SharedEntity;
use std::{fmt::Debug, sync::Arc};
use steel_utils::locks::SyncMutex;

/// A cancellable replacement for a mob's goal, navigation and custom AI decisions.
pub trait MobAiController: Debug + Send + Sync {
    /// Finished controllers retire before the next native AI step.
    fn is_finished(&self) -> bool;
    /// Runs on the mob's normal AI step, without the controller-slot lock held.
    fn tick(&self, entity: &SharedEntity);
    /// Releases controller-specific movement state after retirement.
    fn stop(&self, _entity: &SharedEntity) {}
}

/// One exclusive controller. Installing a new one never replaces active work.
#[derive(Debug, Default)]
pub struct MobAiControllerSlot {
    controller: SyncMutex<Option<Arc<dyn MobAiController>>>,
}
impl MobAiControllerSlot {
    fn snapshot(&self) -> Option<Arc<dyn MobAiController>> {
        self.controller.lock().as_ref().map(Arc::clone)
    }
    /// Whether this mob currently has an unfinished controller.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.snapshot()
            .is_some_and(|controller| !controller.is_finished())
    }
    /// Claims the slot, rejecting concurrent changes and unfinished controllers.
    pub fn install(&self, controller: Arc<dyn MobAiController>) -> Result<(), String> {
        let previous = self.snapshot();
        if previous
            .as_ref()
            .is_some_and(|controller| !controller.is_finished())
        {
            return Err("mob already has an active AI controller".into());
        }
        let mut current = self.controller.lock();
        let unchanged = match (&*current, &previous) {
            (None, None) => true,
            (Some(current), Some(previous)) => Arc::ptr_eq(current, previous),
            _ => false,
        };
        if !unchanged {
            return Err("mob AI controller changed during installation".into());
        }
        *current = Some(controller);
        Ok(())
    }
    pub(crate) fn tick(&self, entity: &SharedEntity) -> bool {
        let Some(controller) = self.snapshot() else {
            return false;
        };
        if controller.is_finished() {
            let mut current = self.controller.lock();
            if current
                .as_ref()
                .is_some_and(|entry| Arc::ptr_eq(entry, &controller))
            {
                *current = None;
            }
            drop(current);
            controller.stop(entity);
            return false;
        }
        controller.tick(entity);
        true
    }
}
