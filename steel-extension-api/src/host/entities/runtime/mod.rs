//! Player-owned entities retain native entity state. Deployed instances are
//! excluded from chunk saves; the player's domain compound is authoritative.

#[cfg(test)]
mod health_tests;
mod ownership;
mod record;
mod release;

use crate::entities::EntityPolicy;
use ownership::StoredEntityBinding;
use steel_core::entity::Entity;

use glam::DVec3;
use simdnbt::owned::NbtTag;
use std::{collections::BTreeMap, mem, sync::Arc};
use steel_core::{
    chunk_saver::ChunkStorage,
    entity::custom_data::DataPath,
    entity::{ENTITIES, LivingEntity, RemovalReason, SharedEntity, attachment::EntityAttachment},
    player::{Player, attachments::PlayerAttachment},
    world::World,
};
use steel_registry::entity_data::EntityPose;
use steel_utils::BlockPos;
use steel_utils::{ChunkPos, DowncastType, DowncastTypeKey, Identifier, locks::SyncMutex};
use uuid::Uuid;

/// Live deployments belonging to the current player incarnation and domain.
#[derive(Default)]
pub struct StoredEntities {
    deployed: SyncMutex<BTreeMap<String, Arc<StoredEntityBinding>>>,
}
// SAFETY: This concrete adapter and its unique key are owned by Steel's SDK host.
unsafe impl DowncastType for EntityStorageService {
    const TYPE_KEY: DowncastTypeKey =
        DowncastTypeKey::new("steel:extension/entity_storage_service");
}
struct EntityStorageService(Arc<StoredEntities>);
impl PlayerAttachment for EntityStorageService {
    fn synchronize(&self, player: &Player) -> Result<(), String> {
        self.0.synchronize(player)
    }
    fn detach(&self, player: &Player) -> Result<(), String> {
        self.0.recall_all(player)
    }
    fn discard(&self) {
        self.0.discard_deployments();
    }
}

impl StoredEntities {
    pub(crate) fn of(player: &Player) -> Result<Arc<Self>, String> {
        use steel_utils::Downcast;
        let attachment = player.attachments.get_or_insert(
            Identifier::from_steel("entity_storage"),
            Arc::new(EntityStorageService(Arc::new(Self::default()))),
        );
        attachment
            .downcast_ref::<EntityStorageService>()
            .map(|service| Arc::clone(&service.0))
            .ok_or_else(|| {
                "entity storage service key is occupied by another implementation".into()
            })
    }

    /// Copies live deployment handles without retaining the ownership-map lock.
    #[must_use]
    pub fn deployments(&self) -> Vec<(String, SharedEntity, Uuid)> {
        self.deployed
            .lock()
            .iter()
            .map(|(path, entry)| (path.clone(), entry.entity(), entry.instance()))
            .collect()
    }
    /// Updates native snapshots atomically while preserving dependency-owned fields.
    pub fn synchronize(&self, player: &Player) -> Result<(), String> {
        let deployed: Vec<_> = self
            .deployed
            .lock()
            .iter()
            .map(|(path, entry)| (path.clone(), entry.entity()))
            .collect();
        if deployed.is_empty() {
            return Ok(());
        }
        let mut updates = Vec::new();
        for (path, entity) in deployed {
            updates.push((path, record::snapshot(&entity)?));
        }
        let mut data = player.data();
        let mut next = data.clone();
        for (path, record) in updates {
            for (name, value) in record {
                let name = name.as_str().to_str();
                if name == "health" {
                    let NbtTag::Compound(gauge) = value else {
                        return Err("native health gauge is not a compound".into());
                    };
                    for (field, value) in gauge {
                        next.set(
                            &DataPath::parse(&format!(
                                "{path}/health/{}",
                                field.as_str().to_str()
                            ))?,
                            value,
                        )?;
                    }
                } else {
                    next.set(&DataPath::parse(&format!("{path}/{name}"))?, value)?;
                }
            }
        }
        *data = next;
        Ok(())
    }

    /// Stores a supported native entity into a vacant compound field.
    pub fn store(
        &self,
        player: &Player,
        entity: &SharedEntity,
        path: &str,
        policy: EntityPolicy,
    ) -> Result<(), String> {
        validate_policy(policy)?;
        let data_path = DataPath::parse(path)?;
        if self.deployed.lock().contains_key(path) {
            return Err("recall the deployed entity before reusing its data slot".into());
        }
        let world = player.get_world();
        if entity
            .level()
            .is_none_or(|level| !Arc::ptr_eq(&level, &world))
        {
            return Err("storage target must be in the owner's current world".into());
        }
        if entity.as_player().is_some() || entity.is_removed() || !entity.is_persistent() {
            return Err("storage requires a serializable native entity".into());
        }
        if entity.is_passenger()
            || !entity.passengers().is_empty()
            || entity
                .as_leashable()
                .is_some_and(|leash| leash.leash_holder().is_some())
        {
            return Err("detach passengers, vehicles and leashes before storage".into());
        }
        if !ENTITIES.supports_loading(entity.entity_type()) {
            return Err("this entity has no native load factory".into());
        }
        let living = entity.as_living_entity();
        let previous_health = living.map(LivingEntity::get_health);
        if policy.minimum_health > 0.0 && previous_health.is_some_and(|health| health <= 0.0) {
            return Err("a positive health policy requires a living storage target".into());
        }
        let deployment = StoredEntityBinding::new(Arc::clone(entity), policy);
        if !entity
            .base()
            .claim_attachment(&(Arc::clone(&deployment) as Arc<dyn EntityAttachment>))
        {
            return Err("entity is already owned or being stored".into());
        }
        if let (Some(living), Some(health)) = (living, previous_health) {
            living.set_health(health);
        }
        let result = (|| {
            let mut record = record::snapshot(entity)?;
            if let Some(value) = record.get_mut("deployed") {
                *value = NbtTag::Byte(0);
            }
            if !player
                .data()
                .initialize(&data_path, NbtTag::Compound(record))?
            {
                return Err("entity slot is occupied".into());
            }
            entity.set_removed(RemovalReason::Discarded);
            Ok(())
        })();
        if result.is_err() {
            entity.base().release_attachment();
            if let (Some(living), Some(health)) = (living, previous_health) {
                living.set_health(health);
            }
        }
        result
    }

    /// Recreates a stored native entity at a prepared, collision-free position.
    pub fn restore(
        &self,
        player: &Player,
        world: &Arc<World>,
        path: &str,
        position: DVec3,
        policy: EntityPolicy,
    ) -> Result<SharedEntity, String> {
        validate_policy(policy)?;
        if self.deployed.lock().contains_key(path) {
            return Err("recall this entity before deploying it again".into());
        }
        if !position.is_finite()
            || !world.is_in_valid_bounds(BlockPos::containing(position.x, position.y, position.z))
            || world
                .chunk_map
                .with_full_chunk(ChunkPos::from_entity_pos(position), |_| ())
                .is_none()
        {
            return Err("deployment position must be finite, in bounds and loaded".into());
        }
        let mut saved = record::read(player, path)?;
        if world
            .get_entity_in_domain_by_uuid(&Uuid::from_bytes(saved.uuid))
            .is_some()
        {
            return Err("this entity already exists in the domain".into());
        }
        saved.pos = position.to_array();
        saved.motion = [0.0; 3];
        let entity = ChunkStorage::persistent_to_entity_at_level(
            &saved,
            ChunkPos::from_entity_pos(position),
            &Arc::downgrade(world),
        )
        .ok_or("stored entity cannot be loaded")?;
        if let Some(living) = entity.as_living_entity()
            && living.get_health() < policy.minimum_health
        {
            if living.get_health() <= 0.0 {
                let absorption = living.get_absorption_amount();
                living.living_base().reset_death_state();
                living.set_absorption_amount(absorption);
                if entity.pose() == EntityPose::Dying {
                    entity.set_pose(EntityPose::Standing);
                }
            }
            living.set_health(policy.minimum_health);
        }
        if !entity.is_free(DVec3::ZERO) {
            return Err("deployment position is obstructed".into());
        }
        let deployment = StoredEntityBinding::new(Arc::clone(&entity), policy);
        if !entity
            .base()
            .claim_attachment(&(Arc::clone(&deployment) as Arc<dyn EntityAttachment>))
        {
            return Err("deployment ownership could not be claimed".into());
        }
        world
            .try_add_entity(Arc::clone(&entity))
            .map_err(|e| e.to_string())?;
        self.deployed
            .lock()
            .insert(path.to_owned(), Arc::clone(&deployment));
        if let Err(error) = self.synchronize(player) {
            self.deployed.lock().remove(path);
            deployment.entity().set_removed(RemovalReason::Discarded);
            return Err(error);
        }
        Ok(deployment.entity())
    }

    /// Saves current state and recalls one deployment. Recall remains possible after death.
    pub fn return_to_storage(&self, player: &Player, path: &str) -> Result<bool, String> {
        self.synchronize(player)?;
        let Some(entry) = self.deployed.lock().remove(path) else {
            return Ok(false);
        };
        player.data().set(
            &DataPath::parse(&format!("{path}/deployed"))?,
            NbtTag::Byte(0),
        )?;
        entry.entity().set_removed(RemovalReason::Discarded);
        Ok(true)
    }

    /// Recalls all deployments after their state has been snapshotted.
    pub fn discard_deployments(&self) {
        let entries = mem::take(&mut *self.deployed.lock());
        for (_, entry) in entries {
            entry.entity().set_removed(RemovalReason::Discarded);
        }
    }
    /// Persists and recalls every deployment before leaving the current domain.
    pub fn recall_all(&self, player: &Player) -> Result<(), String> {
        self.synchronize(player)?;
        let paths: Vec<_> = self.deployed.lock().keys().cloned().collect();
        let mut data = player.data();
        let mut next = data.clone();
        for path in paths {
            next.set(
                &DataPath::parse(&format!("{path}/deployed"))?,
                NbtTag::Byte(0),
            )?;
        }
        *data = next;
        drop(data);
        self.discard_deployments();
        Ok(())
    }
}
impl Drop for StoredEntities {
    fn drop(&mut self) {
        self.discard_deployments();
    }
}

#[cfg(test)]
mod tests;

fn validate_policy(policy: EntityPolicy) -> Result<(), String> {
    if !policy.minimum_health.is_finite() || policy.minimum_health < 0.0 {
        return Err("minimum health must be finite and nonnegative".into());
    }
    Ok(())
}
