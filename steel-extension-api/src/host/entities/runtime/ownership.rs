//! A stable deployment binding follows native replacement conversions and player respawns.
//! Entities hold only a weak reference; the player attachment owns this binding.

use crate::entities::EntityPolicy;
use std::{ptr, sync::Arc};
use steel_core::entity::Entity;
use steel_core::entity::{ENTITIES, EntityBase, SharedEntity, attachment::EntityAttachment};
use steel_core::{entity::custom_data::DataPath, player::Player};
use steel_utils::locks::SyncMutex;

struct State {
    entity: SharedEntity,
}

pub(crate) struct StoredEntityBinding {
    state: SyncMutex<State>,
    policy: EntityPolicy,
    instance: uuid::Uuid,
}

impl StoredEntityBinding {
    pub(super) fn new(entity: SharedEntity, policy: EntityPolicy) -> Arc<Self> {
        Arc::new(Self {
            state: SyncMutex::new(State { entity }),
            policy,
            instance: uuid::Uuid::new_v4(),
        })
    }
    pub(super) const fn instance(&self) -> uuid::Uuid {
        self.instance
    }
    pub(super) fn entity(&self) -> SharedEntity {
        Arc::clone(&self.state.lock().entity)
    }
    pub(super) fn release(&self, player: &Player, path: &DataPath) -> Result<SharedEntity, String> {
        let state = self.state.lock();
        if state.entity.is_removed() {
            return Err("a removed entity cannot be released".into());
        }
        if state
            .entity
            .level()
            .is_none_or(|world| !Arc::ptr_eq(&world, &player.get_world()))
        {
            return Err("entity is not in the owner's current world".into());
        }
        let mut data = player.data();
        if data.get(path).and_then(|value| value.compound()).is_none() {
            return Err("entity has no storage record".into());
        }
        data.remove(path);
        state.entity.base().release_attachment();
        Ok(Arc::clone(&state.entity))
    }
}

impl EntityAttachment for StoredEntityBinding {
    fn save_in_chunk(&self) -> bool {
        false
    }
    fn can_use_portal(&self) -> bool {
        false
    }
    fn drop_loot(&self) -> bool {
        self.policy.drop_loot
    }
    fn health(&self, requested: f32) -> f32 {
        if requested < self.policy.minimum_health {
            self.policy.minimum_health
        } else {
            requested
        }
    }
    fn can_convert(&self, to: &SharedEntity) -> bool {
        to.is_persistent() && ENTITIES.supports_loading(to.entity_type())
    }
    fn replace_converted(self: Arc<Self>, from: &EntityBase, to: &SharedEntity) -> bool {
        let mut state = self.state.lock();
        if !ptr::eq(state.entity.base(), from)
            || state.entity.is_removed()
            || !to
                .base()
                .claim_attachment(&(Arc::clone(&self) as Arc<dyn EntityAttachment>))
        {
            return false;
        }
        state.entity = Arc::clone(to);
        true
    }
}
