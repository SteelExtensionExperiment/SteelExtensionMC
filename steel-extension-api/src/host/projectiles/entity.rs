//! Custom thrown projectiles use vanilla throwable collision and movement, rendered
//! as an item-carrying snowball. Only the impact and lifetime are extension-defined.

use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

use glam::DVec3;
use steel_protocol::packets::game::SoundSource;
use steel_registry::{
    entity_type::EntityTypeRef, item_stack::ItemStack, items::ItemRef, vanilla_entities,
    vanilla_entity_data::SnowballEntityData, vanilla_items,
};
use steel_utils::{DowncastType, DowncastTypeKey, locks::SyncMutex};

use steel_core::{
    entity::{
        Entity, EntityBase, EntitySyncedData, Projectile, ProjectileBase, ProjectileHit,
        RemovalReason, SharedEntity, ThrowableItemProjectile, ThrowableProjectile,
    },
    world::World,
};

/// Receives one synchronous impact on the world's normal tick thread.
pub trait ImpactHandler: Send + Sync {
    /// Runs after native block impact behavior, with no projectile-state lock held.
    fn on_hit(
        &self,
        projectile: &Arc<ExtensionProjectile>,
        world: &Arc<World>,
        hit: &ProjectileHit,
    );
}

/// An ephemeral extension projectile; callbacks are never serialized as vanilla entities.
pub struct ExtensionProjectile {
    base: EntityBase,
    data: SyncMutex<SnowballEntityData>,
    projectile: ProjectileBase,
    gravity: f64,
    remaining: SyncMutex<u32>,
    impacted: AtomicBool,
    handler: Box<dyn ImpactHandler>,
}

// SAFETY: Steel owns this unique key and this concrete implementation.
unsafe impl DowncastType for ExtensionProjectile {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/extension_projectile");
}

impl ExtensionProjectile {
    /// Builds a custom projectile with validated gravity and a finite lifetime in ticks.
    pub fn new(
        id: i32,
        position: DVec3,
        world: Weak<World>,
        gravity: f64,
        lifetime: u32,
        handler: Box<dyn ImpactHandler>,
    ) -> Result<Self, String> {
        if !gravity.is_finite() || gravity < 0.0 || lifetime == 0 {
            return Err(
                "projectile gravity must be finite and nonnegative, and lifetime positive".into(),
            );
        }
        let projectile = Self {
            base: EntityBase::new(id, position, vanilla_entities::SNOWBALL.dimensions, world),
            data: SyncMutex::new(SnowballEntityData::new()),
            projectile: ProjectileBase::new(),
            gravity,
            remaining: SyncMutex::new(lifetime),
            impacted: AtomicBool::new(false),
            handler,
        };
        // Snowball clients predict gravity locally between movement updates.
        projectile.set_no_gravity(gravity == 0.0);
        Ok(projectile)
    }
}

impl Entity for ExtensionProjectile {
    fn base(&self) -> &EntityBase {
        &self.base
    }
    fn entity_type(&self) -> EntityTypeRef {
        &vanilla_entities::SNOWBALL
    }
    fn is_persistent(&self) -> bool {
        false
    }
    fn tick(self: Arc<Self>) {
        if self.is_removed() {
            return;
        }
        Arc::clone(&self).throwable_projectile_tick();
        let expired = {
            let mut remaining = self.remaining.lock();
            *remaining = remaining.saturating_sub(1);
            *remaining == 0
        };
        if expired {
            self.set_removed(RemovalReason::Discarded);
        }
    }
    // The callback has no serialized representation in a recreated destination entity.
    fn can_use_portal(&self, _ignore_passenger: bool) -> bool {
        false
    }
    fn get_default_gravity(&self) -> f64 {
        self.gravity
    }
    fn sound_source(&self) -> SoundSource {
        SoundSource::Neutral
    }
    fn spawn_data(&self) -> i32 {
        self.get_owner().map_or(0, |owner| owner.id())
    }
    fn restore_owner_reference(&self, owner: &SharedEntity) {
        self.cache_owner_entity(owner);
    }
    fn projectile_owner_uuid(&self) -> Option<uuid::Uuid> {
        self.owner_uuid()
    }
    fn projectile_owner(&self) -> Option<SharedEntity> {
        self.get_owner()
    }
    fn synced_data(&self) -> Option<&dyn EntitySyncedData> {
        Some(&self.data)
    }
}

impl Projectile for ExtensionProjectile {
    fn projectile_base(&self) -> &ProjectileBase {
        &self.projectile
    }
    fn on_hit(self: Arc<Self>, hit: &ProjectileHit) {
        if self.impacted.swap(true, Ordering::AcqRel) {
            return;
        }
        Arc::clone(&self).projectile_on_hit(hit);
        self.set_removed(RemovalReason::Discarded);
        if let Some(world) = self.level() {
            self.handler.on_hit(&self, &world, hit);
        }
    }
}
impl ThrowableProjectile for ExtensionProjectile {}
impl ThrowableItemProjectile for ExtensionProjectile {
    fn get_default_item(&self) -> ItemRef {
        &vanilla_items::SNOWBALL
    }
    fn set_item(&self, item: ItemStack) {
        self.data
            .lock()
            .throwable_item_projectile
            .item_stack
            .set(item);
    }
    fn get_item(&self) -> ItemStack {
        self.data
            .lock()
            .throwable_item_projectile
            .item_stack
            .get()
            .clone()
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
