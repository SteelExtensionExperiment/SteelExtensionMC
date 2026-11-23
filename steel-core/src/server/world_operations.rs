//! Server-wide table of asynchronous world operations (create, destroy, template load, remove).
use super::Server;
use rustc_hash::FxHashMap;
use steel_utils::{Identifier, locks::SyncMutex};

/// Identity of an asynchronous world operation, consumed when its result is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorldOperationId(pub u64);

#[derive(Default)]
pub(super) struct WorldOperations {
    state: SyncMutex<State>,
}

#[derive(Default)]
struct State {
    next: u64,
    operations: FxHashMap<WorldOperationId, Option<Result<Identifier, String>>>,
}

impl WorldOperations {
    /// Registers a pending operation. Fails once identities are exhausted.
    pub(super) fn allocate(&self) -> Result<WorldOperationId, String> {
        let mut state = self.state.lock();
        state.next = state
            .next
            .checked_add(1)
            .ok_or("world operation identities exhausted")?;
        let id = WorldOperationId(state.next);
        state.operations.insert(id, None);
        Ok(id)
    }

    /// Stores the result of a pending operation. Unknown ids are ignored.
    pub(super) fn finish(&self, id: WorldOperationId, result: Result<Identifier, String>) {
        if let Some(slot) = self.state.lock().operations.get_mut(&id) {
            *slot = Some(result);
        }
    }

    fn take(&self, id: WorldOperationId) -> Result<Option<Result<Identifier, String>>, String> {
        let mut state = self.state.lock();
        let slot = state
            .operations
            .get_mut(&id)
            .ok_or("unknown or consumed world operation")?;
        if slot.is_none() {
            return Ok(None);
        }
        let result = slot.take();
        state.operations.remove(&id);
        Ok(result)
    }
}

impl Server {
    /// Reads and consumes a completed operation. `None` means its job is still running.
    pub fn take_world_operation(
        &self,
        id: WorldOperationId,
    ) -> Result<Option<Result<Identifier, String>>, String> {
        self.world_operations.take(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_is_consumed_on_first_read() -> Result<(), String> {
        let operations = WorldOperations::default();
        let id = operations.allocate()?;
        assert_eq!(operations.take(id)?, None);
        operations.finish(id, Ok(Identifier::new("a", "b")));
        assert_eq!(operations.take(id)?, Some(Ok(Identifier::new("a", "b"))));
        assert!(operations.take(id).is_err());
        Ok(())
    }

    #[test]
    fn identities_fail_on_overflow() {
        let operations = WorldOperations::default();
        operations.state.lock().next = u64::MAX;
        assert!(operations.allocate().is_err());
    }
}
