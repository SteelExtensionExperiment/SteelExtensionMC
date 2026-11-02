//! Runtime services participating in player saves and incarnation changes.

use super::Player;
use std::{collections::BTreeMap, mem, sync::Arc};
use steel_utils::{ErasedType, Identifier, locks::SyncMutex};

/// A service owned by one player incarnation and transferred on respawn.
pub trait PlayerAttachment: ErasedType + Send + Sync {
    /// Publishes runtime state before the player's persistent snapshot is read.
    fn synchronize(&self, player: &Player) -> Result<(), String>;
    /// Prepares state before disconnect, shutdown, or a domain switch.
    fn detach(&self, player: &Player) -> Result<(), String>;
    /// Releases runtime resources, including after an unsuccessful snapshot.
    fn discard(&self);
}

/// Namespaced services. Callbacks always run outside the collection lock.
#[derive(Default)]
pub struct PlayerAttachments {
    entries: SyncMutex<BTreeMap<Identifier, Arc<dyn PlayerAttachment>>>,
}

impl PlayerAttachments {
    /// Returns the installed service, installing the supplied one if absent.
    pub fn get_or_insert(
        &self,
        key: Identifier,
        service: Arc<dyn PlayerAttachment>,
    ) -> Arc<dyn PlayerAttachment> {
        Arc::clone(self.entries.lock().entry(key).or_insert(service))
    }

    fn snapshot(&self) -> Vec<Arc<dyn PlayerAttachment>> {
        self.entries.lock().values().map(Arc::clone).collect()
    }

    /// Publishes all service state before a player save.
    pub fn synchronize(&self, player: &Player) -> Result<(), String> {
        for service in self.snapshot() {
            service.synchronize(player)?;
        }
        Ok(())
    }

    /// Prepares all services before leaving the current domain.
    pub fn detach(&self, player: &Player) -> Result<(), String> {
        for service in self.snapshot() {
            service.detach(player)?;
        }
        Ok(())
    }

    /// Releases live resources without removing services from this incarnation.
    pub fn discard(&self) {
        for service in self.snapshot() {
            service.discard();
        }
    }

    pub(crate) fn transfer_from(&self, previous: &Self) {
        let entries = mem::take(&mut *previous.entries.lock());
        *self.entries.lock() = entries;
    }
}

impl Drop for PlayerAttachments {
    fn drop(&mut self) {
        self.discard();
    }
}
