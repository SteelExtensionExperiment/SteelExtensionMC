//! Removal of a single runtime world, relocating its players first.
use super::{Server, State};
use crate::{
    portal::WorldChangeRequest,
    server::{
        PlayerAdmissionState, WorldEntry,
        jobs::{JobPoll, ServerJob, ServerJobContext},
        world_operations::WorldOperationId,
    },
    world::World,
};
use std::sync::{Arc, Weak};
use steel_utils::Identifier;

impl Server {
    /// Starts removing a runtime world. From this call on the world refuses new entries; its
    /// players are moved to the domain default world, else the server default world, and the
    /// operation finishes with the world's key once it is retired.
    ///
    /// # Errors
    /// Fails immediately, without an operation, for `worlds.toml` worlds, worlds that are being
    /// created or removed, closing domains, and clock owners that other worlds still share.
    pub fn remove_world(self: &Arc<Self>, key: &Identifier) -> Result<WorldOperationId, String> {
        let domain = key.namespace.to_string();
        let (id, world) = {
            let mut state = self.temporary_worlds.state.lock();
            let world = self.check_removable(&state, key)?;
            let id = self.world_operations.allocate()?;
            if let Some(entry) = state.domains.get_mut(&domain) {
                entry.removing += 1;
            }
            world.begin_retiring();
            (id, world)
        };
        self.jobs.spawn(RemoveWorldJob {
            server: Arc::downgrade(self),
            id,
            domain,
            world,
        });
        Ok(id)
    }

    /// Whether [`Server::remove_world`] would accept `key` right now.
    #[must_use]
    pub fn world_is_removable(&self, key: &Identifier) -> bool {
        let state = self.temporary_worlds.state.lock();
        self.check_removable(&state, key).is_ok()
    }

    fn check_removable(&self, state: &State, key: &Identifier) -> Result<Arc<World>, String> {
        if state.pending_worlds.contains(key) {
            return Err(format!("world {key} is still being created"));
        }
        let world = self
            .worlds
            .get(key)
            .ok_or_else(|| format!("world {key} is not loaded"))?;
        if !self.worlds.is_runtime_world(key) {
            return Err(format!(
                "world {key} is configured in worlds.toml and cannot be removed"
            ));
        }
        if world.is_retiring() {
            return Err(format!("world {key} is already being removed"));
        }
        let domain = key.namespace.as_ref();
        if state.domains.get(domain).is_some_and(|d| d.closing) {
            return Err("temporary domain is closing".into());
        }
        if self
            .worlds
            .worlds_in_domain(domain)
            .iter()
            .any(|other| other.key != *key && Arc::ptr_eq(&other.game_time, &world.game_time))
        {
            return Err(format!(
                "the clock of world {key} is shared by other worlds"
            ));
        }
        Ok(world)
    }
}

struct RemoveWorldJob {
    server: Weak<Server>,
    id: WorldOperationId,
    domain: String,
    world: Arc<World>,
}

impl RemoveWorldJob {
    /// Domain default world, else the server default world; never this world or a retiring one.
    fn destination(&self, server: &Server) -> Option<Arc<World>> {
        let usable = |world: &Arc<World>| !world.is_retiring() && world.key != self.world.key;
        server
            .worlds
            .default_world(&self.domain)
            .filter(usable)
            .or_else(|| server.worlds.server_default_world().filter(usable))
    }

    fn is_targeted(&self, server: &Server) -> bool {
        let selection_pending = server
            .pending_world_changes
            .lock()
            .iter()
            .any(|(_, request)| {
                matches!(request, WorldChangeRequest::WorldSpawn(selection)
                    if Arc::ptr_eq(&selection.target_world, &self.world))
            });
        selection_pending
            || server.pending_domain_switches.lock().iter().any(|switch| {
                switch
                    .target_world
                    .as_ref()
                    .is_some_and(|target| Arc::ptr_eq(target, &self.world))
            })
    }

    fn finish_removal(&self, server: &Arc<Server>) {
        if let Some(entry) = server
            .temporary_worlds
            .state
            .lock()
            .domains
            .get_mut(&self.domain)
        {
            entry.removing -= 1;
        }
    }
}

impl ServerJob for RemoveWorldJob {
    fn poll(&mut self, _: &mut ServerJobContext) -> JobPoll {
        let Some(server) = self.server.upgrade() else {
            return JobPoll::Finished;
        };
        let destination = self.destination(&server);
        let mut players_inside = false;
        for player in server.get_players() {
            if player.get_world().key != self.world.key {
                continue;
            }
            players_inside = true;
            if let Some(destination) = &destination
                && let Err(error) =
                    server.queue_player_world_selection(player, destination, WorldEntry::Usual)
            {
                // Transient (relocation already running, player dead); retried on the next poll.
                log::trace!("Waiting to relocate out of {}: {error}", self.world.key);
            }
        }
        if players_inside
            || self.is_targeted(&server)
            || server.player_admissions.lock().values().any(|state| {
                matches!(
                    state,
                    PlayerAdmissionState::Relocating | PlayerAdmissionState::Disconnecting
                )
            })
        {
            return JobPoll::Pending;
        }
        let Some(world) = server.worlds.remove_runtime_world(&self.world.key) else {
            self.finish_removal(&server);
            server
                .world_operations
                .finish(self.id, Err("world was already removed".into()));
            return JobPoll::Finished;
        };
        server
            .temporary_worlds
            .state
            .lock()
            .spawn_tickets
            .remove(&world.key);
        crate::extension::Events::fire_if(&server.events().world_unload, || {
            crate::extension::events::WorldUnloadEvent {
                world: world.key.clone(),
            }
        });
        self.finish_removal(&server);
        let id = self.id;
        let key = world.key.clone();
        server.temporary_worlds.cleanup.clone().spawn(async move {
            world.retire_runtime().await;
            server.world_operations.finish(id, Ok(key));
        });
        JobPoll::Finished
    }

    fn cancel(&mut self) {
        if let Some(server) = self.server.upgrade() {
            self.finish_removal(&server);
            server
                .world_operations
                .finish(self.id, Err("world removal cancelled".into()));
        }
    }
}
