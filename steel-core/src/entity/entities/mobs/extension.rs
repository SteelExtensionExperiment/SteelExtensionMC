//! Mobs registered through `client_content`, carried to clients by a vanilla type.
//!
//! One [`ExtensionMob`] implementation serves every registered type. Its synced data
//! uses the carrier's vanilla layout, so clients render the carrier. Its behavior is
//! a host-implemented vanilla style: slime movement for cube carriers, or zombie-like
//! melee for humanoids. Owners add gameplay through [`MobHooks`].

use std::sync::{Arc, OnceLock, Weak};

use glam::DVec3;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompoundView;
use simdnbt::owned::NbtCompound;
use steel_protocol::packets::game::SoundSource;
use steel_registry::client_content::{ClientEntity, MobCarrier, MobStyle};
use steel_registry::entity_data::DataValue;
use steel_registry::entity_type::{EntityDimensions, EntityTypeRef};
use steel_registry::sound_event::SoundEventRef;
use steel_registry::vanilla_entity_data::{
    BaseEntityData, HuskEntityData, LivingEntityData, MagmaCubeEntityData, SlimeEntityData,
    VanillaEntityData, VanillaLivingEntityData, ZombieEntityData,
};
use steel_registry::{REGISTRY, sound_events, vanilla_attributes, vanilla_damage_types};
use steel_utils::locks::SyncMutex;
use steel_utils::{BlockPos, BlockStateId, DowncastType, DowncastTypeKey};

use crate::entity::ai::goal::{
    FloatGoal, HurtByTargetGoal, LookAtPlayerGoal, MeleeAttackGoal, NearestAttackableTargetGoal,
    Goal, GoalControls, RandomLookAroundGoal, WaterAvoidingRandomStrollGoal,
};
use crate::entity::damage::DamageSource;
use crate::entity::{
    Entity, EntityBase, EntityBaseLoad, EntityMovementEmission, EntityPose, EntitySyncedData,
    LivingEntity, LivingEntityBase, LivingTravelInput, Mob, MobBase, Monster, PathfinderMob,
    RemovalReason, SharedEntity, sync_dirty_mob_effects,
};
use crate::physics::MoveResult;
use crate::player::Player;
use crate::world::World;

/// Gameplay hooks an owner attaches to a registered mob type.
///
/// Every hook runs on the tick thread of the world the mob is in.
pub trait MobHooks: Send + Sync {
    /// Runs at the end of each of the mob's AI ticks.
    fn tick(&self, world: &Arc<World>, mob: &dyn Entity);
    /// The mob hurt `target` with a melee or contact attack.
    fn attacked(&self, world: &Arc<World>, mob: &dyn Entity, target: &dyn Entity);
    /// The mob was killed; it is still in the world at its final position.
    fn died(&self, world: &Arc<World>, mob: &dyn Entity);
}

type RegistrationHook = Box<dyn FnOnce(&mut MobHookRegistry) + Send>;
static REGISTRATION_HOOKS: SyncMutex<Option<Vec<RegistrationHook>>> =
    SyncMutex::new(Some(Vec::new()));
static MOB_HOOKS: OnceLock<MobHookRegistry> = OnceLock::new();

/// Hooks per registered mob type, filled once while entity factories initialize.
#[derive(Default)]
pub struct MobHookRegistry {
    hooks: Vec<(EntityTypeRef, Arc<dyn MobHooks>)>,
}

impl MobHookRegistry {
    /// Installs hook edits before mob hooks are published, like block behaviors.
    ///
    /// # Errors
    /// Returns an error if mob hooks were already initialized.
    pub fn on_registration(hook: impl FnOnce(&mut Self) + Send + 'static) -> Result<(), String> {
        let mut hooks = REGISTRATION_HOOKS.lock();
        let Some(hooks) = hooks.as_mut() else {
            return Err("mob hook registration has already finished".into());
        };
        hooks.push(Box::new(hook));
        Ok(())
    }

    /// Attaches hooks to a registered mob type. A type accepts hooks once.
    ///
    /// # Errors
    /// Returns an error if the key is not a registered mob type or already has hooks.
    pub fn set_hooks(
        &mut self,
        key: &steel_utils::Identifier,
        hooks: Arc<dyn MobHooks>,
    ) -> Result<(), String> {
        use steel_registry::RegistryExt as _;
        let entity_type = REGISTRY
            .entity_types
            .by_key(key)
            .filter(|entity_type| REGISTRY.client_content.entity(entity_type).is_some())
            .ok_or_else(|| format!("{key} is not a registered extension mob"))?;
        if self.hooks.iter().any(|(existing, _)| std::ptr::eq(*existing, entity_type)) {
            return Err(format!("{key} already has mob hooks"));
        }
        self.hooks.push((entity_type, hooks));
        Ok(())
    }

    fn get(&self, entity_type: EntityTypeRef) -> Option<&Arc<dyn MobHooks>> {
        self.hooks
            .iter()
            .find(|(existing, _)| std::ptr::eq(*existing, entity_type))
            .map(|(_, hooks)| hooks)
    }
}

/// Runs pending hook registrations once and publishes the result.
pub(crate) fn init_mob_hooks() {
    MOB_HOOKS.get_or_init(|| {
        let mut registry = MobHookRegistry::default();
        if let Some(hooks) = REGISTRATION_HOOKS.lock().take() {
            for hook in hooks {
                hook(&mut registry);
            }
        }
        registry
    });
}

fn hooks_for(entity_type: EntityTypeRef) -> Option<&'static Arc<dyn MobHooks>> {
    MOB_HOOKS.get().and_then(|registry| registry.get(entity_type))
}

/// The carrier's synced data, so clients decode it as that vanilla type.
#[derive(Debug, Clone)]
enum CarrierData {
    Slime(SlimeEntityData),
    MagmaCube(MagmaCubeEntityData),
    Zombie(ZombieEntityData),
    Husk(HuskEntityData),
}

macro_rules! each_carrier {
    ($value:expr, $data:ident => $body:expr) => {
        match $value {
            CarrierData::Slime($data) => $body,
            CarrierData::MagmaCube($data) => $body,
            CarrierData::Zombie($data) => $body,
            CarrierData::Husk($data) => $body,
        }
    };
}

impl CarrierData {
    fn new(carrier: MobCarrier) -> Self {
        match carrier {
            MobCarrier::Slime => Self::Slime(SlimeEntityData::new()),
            MobCarrier::MagmaCube => Self::MagmaCube(MagmaCubeEntityData::new()),
            MobCarrier::Zombie => Self::Zombie(ZombieEntityData::new()),
            MobCarrier::Husk => Self::Husk(HuskEntityData::new()),
        }
    }

    fn set_cube_size(&mut self, size: i32) {
        match self {
            Self::Slime(data) => data.abstract_cube_mob_mut().id_size.set(size),
            Self::MagmaCube(data) => data.abstract_cube_mob_mut().id_size.set(size),
            Self::Zombie(_) | Self::Husk(_) => {}
        }
    }

    fn mob_flags(&self) -> i8 {
        each_carrier!(self, data => *data.mob().mob_flags.get())
    }

    fn set_mob_flags(&mut self, flags: i8) {
        each_carrier!(self, data => data.mob_mut().mob_flags.set(flags));
    }
}

impl VanillaEntityData for CarrierData {
    fn base(&self) -> &BaseEntityData {
        each_carrier!(self, data => data.base())
    }

    fn base_mut(&mut self) -> &mut BaseEntityData {
        each_carrier!(self, data => data.base_mut())
    }

    fn pack_dirty(&mut self) -> Option<Vec<DataValue>> {
        each_carrier!(self, data => data.pack_dirty())
    }

    fn pack_all(&self) -> Vec<DataValue> {
        each_carrier!(self, data => data.pack_all())
    }

    fn is_dirty(&self) -> bool {
        each_carrier!(self, data => data.is_dirty())
    }
}

impl VanillaLivingEntityData for CarrierData {
    fn living_entity(&self) -> &LivingEntityData {
        each_carrier!(self, data => data.living_entity())
    }

    fn living_entity_mut(&mut self) -> &mut LivingEntityData {
        each_carrier!(self, data => data.living_entity_mut())
    }
}

/// Vanilla `SlimeMoveControl` state, shared with the slime goals that steer it.
#[derive(Debug, Clone, Copy)]
struct HopControl {
    /// Facing the slime turns toward.
    y_rot: f32,
    /// Ticks until the next hop.
    jump_delay: i32,
    /// Aggressive slimes hop three times as often.
    aggressive: bool,
    /// Requested movement speed, or `None` to stand still (vanilla `WAIT`).
    wanted_speed: Option<f64>,
}

impl HopControl {
    const fn new() -> Self {
        Self {
            y_rot: 0.0,
            jump_delay: 0,
            aggressive: false,
            wanted_speed: None,
        }
    }

    const fn set_direction(&mut self, y_rot: f32, aggressive: bool) {
        self.y_rot = y_rot;
        self.aggressive = aggressive;
    }

    const fn set_wanted_movement(&mut self, speed: f64) {
        self.wanted_speed = Some(speed);
    }
}

type SharedHop = Arc<SyncMutex<HopControl>>;

/// Vanilla `Slime.getJumpDelay`.
fn jump_delay() -> i32 {
    rand::random_range(10..30)
}

/// A registered mob. Not annotated with `#[entity_behavior]`: its factories are
/// installed per registered type, never for a vanilla class.
pub struct ExtensionMob {
    base: EntityBase,
    entity_type: EntityTypeRef,
    definition: &'static ClientEntity,
    living_base: LivingEntityBase,
    mob_base: MobBase,
    entity_data: SyncMutex<CarrierData>,
    hop: SharedHop,
}

// SAFETY: The owner-scoped type key uniquely identifies ExtensionMob.
unsafe impl DowncastType for ExtensionMob {
    const TYPE_KEY: DowncastTypeKey = DowncastTypeKey::new("steel:entity/extension_mob");
}

impl ExtensionMob {
    /// Creates a new instance of a registered mob type.
    ///
    /// # Panics
    /// Panics if `entity_type` is not a registered mob type; factories are only
    /// installed for registered types.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        let dimensions = Self::base_dimensions(entity_type);
        Self::new_with_base(EntityBase::new(id, position, dimensions, world), entity_type)
    }

    /// Loads a saved instance of a registered mob type.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        let dimensions = Self::base_dimensions(entity_type);
        Self::new_with_base(EntityBase::from_load(load, dimensions), entity_type)
    }

    fn definition(entity_type: EntityTypeRef) -> &'static ClientEntity {
        let Some(definition) = REGISTRY.client_content.entity(entity_type) else {
            panic!("{} is not a registered extension mob", entity_type.key);
        };
        definition
    }

    /// Vanilla slimes scale their dimensions by their size.
    fn base_dimensions(entity_type: EntityTypeRef) -> EntityDimensions {
        match Self::definition(entity_type).style {
            MobStyle::Cube { size } => entity_type.dimensions.scale(size as f32),
            MobStyle::Melee => entity_type.dimensions,
        }
    }

    fn new_with_base(base: EntityBase, entity_type: EntityTypeRef) -> Self {
        let definition = Self::definition(entity_type);
        let living_base = LivingEntityBase::new(entity_type);
        let mob_base = MobBase::new();
        let mut entity_data = CarrierData::new(definition.carrier);
        living_base.initialize_synced_data(&mut entity_data);
        entity_data
            .base_mut()
            .custom_name
            .set(Some(Box::new(definition.name.clone())));
        entity_data
            .base_mut()
            .custom_name_visible
            .set(definition.name_visible);
        let hop: SharedHop = Arc::new(SyncMutex::new(HopControl::new()));
        match definition.style {
            MobStyle::Cube { size } => {
                entity_data.set_cube_size(size);
                mob_base.set_xp_reward(size);
                Self::add_cube_goals(&mob_base, &hop);
            }
            MobStyle::Melee => {
                mob_base.set_xp_reward(5);
                Self::add_melee_goals(&mob_base);
            }
        }
        let mob = Self {
            base,
            entity_type,
            definition,
            living_base,
            mob_base,
            entity_data: SyncMutex::new(entity_data),
            hop,
        };
        for (slot, item) in &definition.equipment {
            mob.living_base.equipment().lock().set(*slot, item.clone());
            mob.set_equipment_drop_chance(*slot, 0.0);
        }
        mob
    }

    /// Vanilla `Slime.registerGoals`.
    fn add_cube_goals(mob_base: &MobBase, hop: &SharedHop) {
        let mut goals = mob_base.goal_selector().lock();
        goals.add_goal(1, SlimeFloatGoal(Arc::clone(hop)));
        goals.add_goal(2, SlimeAttackGoal::new(Arc::clone(hop)));
        goals.add_goal(3, SlimeRandomDirectionGoal::new(Arc::clone(hop)));
        goals.add_goal(5, SlimeKeepOnJumpingGoal(Arc::clone(hop)));
        let mut targets = mob_base.target_selector().lock();
        targets.add_goal(
            1,
            NearestAttackableTargetGoal::new_for_players(true, |target, mob_world| {
                let _ = mob_world;
                target.as_player().is_some()
            }),
        );
    }

    /// Zombie-like melee, as vanilla `Zombie.registerGoals` without its special goals.
    fn add_melee_goals(mob_base: &MobBase) {
        let mut goals = mob_base.goal_selector().lock();
        goals.add_goal(1, FloatGoal::new(mob_base));
        goals.add_goal(2, MeleeAttackGoal::new(1.0, false));
        goals.add_goal(7, WaterAvoidingRandomStrollGoal::new(1.0));
        goals.add_goal(8, LookAtPlayerGoal::new(8.0));
        goals.add_goal(8, RandomLookAroundGoal::new());
        let mut targets = mob_base.target_selector().lock();
        targets.add_goal(1, HurtByTargetGoal::new().set_alert_others(std::iter::empty()));
        targets.add_goal(2, NearestAttackableTargetGoal::new_for_players(true, |_, _| true));
    }

    const fn cube_size(&self) -> Option<i32> {
        match self.definition.style {
            MobStyle::Cube { size } => Some(size),
            MobStyle::Melee => None,
        }
    }

    fn is_magma(&self) -> bool {
        self.definition.carrier == MobCarrier::MagmaCube
    }

    fn update_dirty_mob_effect_entity_data(&self) {
        sync_dirty_mob_effects(&self.living_base, &self.entity_data);
    }

    /// Vanilla `SlimeMoveControl.tick`.
    fn tick_hop_control(&self) {
        let control = {
            let mut hop = self.hop.lock();
            let control = *hop;
            hop.wanted_speed = None;
            control
        };
        let (yaw, pitch) = self.rotation();
        let yaw = rotlerp(yaw, control.y_rot, 90.0);
        self.set_rotation((yaw, pitch));
        self.set_y_head_rot(yaw);
        self.set_y_body_rot(yaw);
        let Some(speed_modifier) = control.wanted_speed else {
            let input = self.travel_input();
            self.set_travel_input(LivingTravelInput::new(input.sideways(), input.vertical(), 0.0));
            return;
        };
        let speed = (speed_modifier * self.movement_speed()) as f32;
        if !self.on_ground() {
            self.set_mob_speed(speed);
            return;
        }
        self.set_mob_speed(speed);
        let jump = {
            let mut hop = self.hop.lock();
            hop.jump_delay -= 1;
            if hop.jump_delay > 0 {
                false
            } else {
                hop.jump_delay = jump_delay();
                if hop.aggressive {
                    hop.jump_delay /= 3;
                }
                true
            }
        };
        if jump {
            self.mob_base().controls().lock().jump_control.jump();
            self.play_sound(self.jump_sound(), self.sound_volume(), self.cube_pitch());
        } else {
            self.set_travel_input(LivingTravelInput::ZERO);
            self.set_mob_speed(0.0);
        }
    }

    fn movement_speed(&self) -> f64 {
        self.attributes()
            .lock()
            .get_value(vanilla_attributes::MOVEMENT_SPEED)
            .unwrap_or(0.0)
    }

    fn attack_damage(&self) -> f32 {
        self.attributes()
            .lock()
            .get_value(vanilla_attributes::ATTACK_DAMAGE)
            .unwrap_or(0.0) as f32
    }

    fn is_tiny(&self) -> bool {
        self.cube_size().is_some_and(|size| size <= 1)
    }

    /// Vanilla `Slime.getSoundPitch`.
    fn cube_pitch(&self) -> f32 {
        let tiny = if self.is_tiny() { 1.4 } else { 0.8 };
        ((rand::random::<f32>() - rand::random::<f32>()) * 0.2 + 1.0) * tiny
    }

    fn jump_sound(&self) -> SoundEventRef {
        match (self.is_magma(), self.is_tiny()) {
            (true, _) => &sound_events::ENTITY_MAGMA_CUBE_JUMP,
            (false, true) => &sound_events::ENTITY_SLIME_JUMP_SMALL,
            (false, false) => &sound_events::ENTITY_SLIME_JUMP,
        }
    }

    fn notify_attacked(&self, target: &dyn Entity) {
        if let (Some(hooks), Some(world)) = (hooks_for(self.entity_type), self.level()) {
            hooks.attacked(&world, self, target);
        }
    }
}

/// Vanilla `Mth.rotLerp` toward a target yaw with a maximum step.
fn rotlerp(from: f32, to: f32, max_step: f32) -> f32 {
    let mut delta = (to - from) % 360.0;
    if delta >= 180.0 {
        delta -= 360.0;
    }
    if delta < -180.0 {
        delta += 360.0;
    }
    from + delta.clamp(-max_step, max_step)
}

impl Entity for ExtensionMob {
    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn movement_emission(&self) -> EntityMovementEmission {
        EntityMovementEmission::Events
    }

    fn base_tick(&self) {
        Mob::base_tick_mob(self);
    }

    fn dimensions_for_pose(&self, _pose: EntityPose) -> EntityDimensions {
        let base = Self::base_dimensions(self.entity_type);
        if self.entity_type.fixed {
            base
        } else {
            base.scale(LivingEntity::get_scale(self))
        }
    }

    fn synced_data(&self) -> Option<&dyn EntitySyncedData> {
        Some(&self.entity_data)
    }

    fn update_data_before_sync(&self) {
        self.update_dirty_mob_effect_entity_data();
    }

    fn max_up_step(&self) -> f32 {
        self.attributes()
            .lock()
            .get_value(vanilla_attributes::STEP_HEIGHT)
            .unwrap_or(0.6) as f32
    }

    fn sound_source(&self) -> SoundSource {
        Monster::sound_source_monster(self)
    }

    fn fall_sounds(&self) -> (SoundEventRef, SoundEventRef) {
        Monster::fall_sounds_monster(self)
    }

    fn swim_sound(&self) -> SoundEventRef {
        Monster::swim_sound_monster(self)
    }

    fn swim_splash_sound(&self) -> SoundEventRef {
        Monster::swim_splash_sound_monster(self)
    }

    fn play_step_sound(&self, _pos: BlockPos, _block_state: BlockStateId) {
        match self.definition.carrier {
            MobCarrier::Husk => self.play_sound(&sound_events::ENTITY_HUSK_STEP, 0.15, 1.0),
            MobCarrier::Zombie => self.play_sound(&sound_events::ENTITY_ZOMBIE_STEP, 0.15, 1.0),
            // Cube mobs make their noise when they land instead.
            MobCarrier::Slime | MobCarrier::MagmaCube => {}
        }
    }

    /// Vanilla `Slime.playerTouch`: big cube mobs hurt the players they touch.
    fn player_touch(self: Arc<Self>, player: &Arc<Player>) {
        let Some(size) = self.cube_size() else {
            return;
        };
        if self.is_tiny() || !Entity::is_alive(self.as_ref()) {
            return;
        }
        let reach = 0.6 * f64::from(size);
        if self.position().distance_squared(player.position()) >= reach * reach
            || !self.has_line_of_sight(player.as_ref())
        {
            return;
        }
        let Some(world) = self.level() else {
            return;
        };
        let attacker: SharedEntity = Arc::clone(&self) as SharedEntity;
        let source = DamageSource::environment(&vanilla_damage_types::MOB_ATTACK)
            .with_causing_entity(Arc::clone(&attacker))
            .with_direct_entity(attacker);
        if player.hurt(&world, &source, self.attack_damage()) {
            self.play_sound(&sound_events::ENTITY_SLIME_ATTACK, 1.0, self.cube_pitch());
            self.notify_attacked(player.as_ref());
        }
    }

    fn set_removed(&self, reason: RemovalReason) {
        let first_kill = reason == RemovalReason::Killed && !self.is_removed();
        if first_kill
            && let (Some(hooks), Some(world)) = (hooks_for(self.entity_type), self.level())
        {
            hooks.died(&world, self);
        }
        self.base().set_removed(reason);
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        self.save_mob(nbt);
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        self.load_mob(nbt);
    }
}

impl LivingEntity for ExtensionMob {
    fn living_base(&self) -> &LivingEntityBase {
        &self.living_base
    }

    fn get_health(&self) -> f32 {
        *self.entity_data.lock().living_entity().health.get()
    }

    fn set_health(&self, health: f32) {
        let clamped = health.clamp(0.0, self.get_max_health());
        self.entity_data
            .lock()
            .living_entity_mut()
            .health
            .set(clamped);
    }

    fn sound_volume(&self) -> f32 {
        self.cube_size()
            .map_or(1.0, |size| 0.4 * size as f32)
    }

    fn hurt_sound(&self, _source: &DamageSource) -> Option<SoundEventRef> {
        Some(match (self.definition.carrier, self.is_tiny()) {
            (MobCarrier::Slime, true) => &sound_events::ENTITY_SLIME_HURT_SMALL,
            (MobCarrier::Slime, false) => &sound_events::ENTITY_SLIME_HURT,
            (MobCarrier::MagmaCube, true) => &sound_events::ENTITY_MAGMA_CUBE_HURT_SMALL,
            (MobCarrier::MagmaCube, false) => &sound_events::ENTITY_MAGMA_CUBE_HURT,
            (MobCarrier::Zombie, _) => &sound_events::ENTITY_ZOMBIE_HURT,
            (MobCarrier::Husk, _) => &sound_events::ENTITY_HUSK_HURT,
        })
    }

    fn death_sound(&self) -> Option<SoundEventRef> {
        Some(match (self.definition.carrier, self.is_tiny()) {
            (MobCarrier::Slime, true) => &sound_events::ENTITY_SLIME_DEATH_SMALL,
            (MobCarrier::Slime, false) => &sound_events::ENTITY_SLIME_DEATH,
            (MobCarrier::MagmaCube, true) => &sound_events::ENTITY_MAGMA_CUBE_DEATH_SMALL,
            (MobCarrier::MagmaCube, false) => &sound_events::ENTITY_MAGMA_CUBE_DEATH,
            (MobCarrier::Zombie, _) => &sound_events::ENTITY_ZOMBIE_DEATH,
            (MobCarrier::Husk, _) => &sound_events::ENTITY_HUSK_DEATH,
        })
    }

    fn should_drop_experience(&self) -> bool {
        Monster::should_drop_experience_monster(self)
    }

    fn should_drop_loot(&self, world: &World) -> bool {
        Monster::should_drop_loot_monster(self, world)
    }

    fn server_ai_step(&self, entity: &SharedEntity) {
        Mob::mob_server_ai_step(self, entity);
        if let (Some(hooks), Some(world)) = (hooks_for(self.entity_type), self.level()) {
            hooks.tick(&world, self);
        }
    }

    fn ai_step(&self, entity: &SharedEntity) -> Option<MoveResult> {
        Monster::monster_ai_step(self, entity)
    }
}

impl Mob for ExtensionMob {
    fn mob_base(&self) -> &MobBase {
        &self.mob_base
    }

    fn tick_goal_selectors(&self, entity: &SharedEntity) {
        PathfinderMob::tick_pathfinder_goal_selectors(self, entity);
    }

    fn tick_path_navigation(&self) {
        PathfinderMob::tick_pathfinder_path_navigation(self);
    }

    fn tick_move_control(&self) {
        if self.cube_size().is_some() {
            self.tick_hop_control();
        } else {
            self.tick_vanilla_move_control();
        }
    }

    fn do_hurt_target(&self, entity: &SharedEntity, world: &World, target: &SharedEntity) -> bool {
        let hurt = self.vanilla_do_hurt_target(entity, world, target);
        if hurt {
            self.notify_attacked(target.as_ref());
        }
        hurt
    }

    fn ambient_sound(&self) -> Option<SoundEventRef> {
        match self.definition.carrier {
            MobCarrier::Zombie => Some(&sound_events::ENTITY_ZOMBIE_AMBIENT),
            MobCarrier::Husk => Some(&sound_events::ENTITY_HUSK_AMBIENT),
            MobCarrier::Slime | MobCarrier::MagmaCube => None,
        }
    }

    fn mob_flags(&self) -> i8 {
        self.entity_data.lock().mob_flags()
    }

    fn set_mob_flags(&self, flags: i8) {
        self.entity_data.lock().set_mob_flags(flags);
    }
}

impl Monster for ExtensionMob {}

impl PathfinderMob for ExtensionMob {}

/// Vanilla `Slime.SlimeFloatGoal`.
struct SlimeFloatGoal(SharedHop);

impl Goal for SlimeFloatGoal {
    fn controls(&self) -> GoalControls {
        GoalControls::JUMP | GoalControls::MOVE
    }

    fn can_use(&mut self, mob: &dyn PathfinderMob) -> bool {
        mob.is_in_water() || mob.is_in_lava()
    }

    fn requires_update_every_tick(&self) -> bool {
        true
    }

    fn tick(&mut self, mob: &dyn PathfinderMob, _entity: &SharedEntity) {
        if rand::random::<f32>() < 0.8 {
            mob.mob_base().controls().lock().jump_control.jump();
        }
        self.0.lock().set_wanted_movement(1.2);
    }
}

/// Vanilla `Slime.SlimeAttackGoal`: turn toward the target for up to 300 ticks.
struct SlimeAttackGoal {
    hop: SharedHop,
    grow_tired_timer: i32,
}

impl SlimeAttackGoal {
    const fn new(hop: SharedHop) -> Self {
        Self {
            hop,
            grow_tired_timer: 0,
        }
    }
}

impl Goal for SlimeAttackGoal {
    fn controls(&self) -> GoalControls {
        GoalControls::LOOK
    }

    fn can_use(&mut self, mob: &dyn PathfinderMob) -> bool {
        mob.target().is_some_and(|target| target.is_alive())
    }

    fn start(&mut self, _mob: &dyn PathfinderMob) {
        self.grow_tired_timer = 300;
    }

    fn can_continue_to_use(&mut self, mob: &dyn PathfinderMob) -> bool {
        self.grow_tired_timer -= 1;
        self.grow_tired_timer > 0 && self.can_use(mob)
    }

    fn requires_update_every_tick(&self) -> bool {
        true
    }

    fn tick(&mut self, mob: &dyn PathfinderMob, _entity: &SharedEntity) {
        let Some(target) = mob.target() else {
            return;
        };
        let delta = target.position() - mob.position();
        let yaw = (delta.z.atan2(delta.x).to_degrees() as f32) - 90.0;
        mob.mob_base()
            .controls()
            .lock()
            .look_control
            .set_look_at(target.position(), 10.0, 10.0);
        let deals_damage = mob.mob_base().xp_reward() > 1;
        self.hop.lock().set_direction(yaw, deals_damage);
    }
}

/// Vanilla `Slime.SlimeRandomDirectionGoal`.
struct SlimeRandomDirectionGoal {
    hop: SharedHop,
    chosen_degrees: f32,
    next_randomize_time: i32,
}

impl SlimeRandomDirectionGoal {
    const fn new(hop: SharedHop) -> Self {
        Self {
            hop,
            chosen_degrees: 0.0,
            next_randomize_time: 0,
        }
    }
}

impl Goal for SlimeRandomDirectionGoal {
    fn controls(&self) -> GoalControls {
        GoalControls::LOOK
    }

    fn can_use(&mut self, mob: &dyn PathfinderMob) -> bool {
        mob.target().is_none() && (mob.on_ground() || mob.is_in_water() || mob.is_in_lava())
    }

    fn tick(&mut self, _mob: &dyn PathfinderMob, _entity: &SharedEntity) {
        self.next_randomize_time -= 1;
        if self.next_randomize_time <= 0 {
            self.next_randomize_time = rand::random_range(40..100);
            self.chosen_degrees = rand::random_range(0.0..360.0);
        }
        self.hop.lock().set_direction(self.chosen_degrees, false);
    }
}

/// Vanilla `Slime.SlimeKeepOnJumpingGoal`.
struct SlimeKeepOnJumpingGoal(SharedHop);

impl Goal for SlimeKeepOnJumpingGoal {
    fn controls(&self) -> GoalControls {
        GoalControls::JUMP | GoalControls::MOVE
    }

    fn can_use(&mut self, mob: &dyn PathfinderMob) -> bool {
        !mob.is_passenger()
    }

    fn tick(&mut self, _mob: &dyn PathfinderMob, _entity: &SharedEntity) {
        self.0.lock().set_wanted_movement(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotlerp_takes_the_short_way_and_caps_the_step() {
        assert!((rotlerp(0.0, 30.0, 90.0) - 30.0).abs() < f32::EPSILON);
        assert!((rotlerp(10.0, 350.0, 90.0) - -10.0).abs() < 1e-4);
        assert!((rotlerp(0.0, 170.0, 90.0) - 90.0).abs() < f32::EPSILON);
    }

    #[test]
    fn hop_control_requests_are_one_shot() {
        let mut hop = HopControl::new();
        hop.set_wanted_movement(1.0);
        hop.set_direction(45.0, true);
        assert_eq!(hop.wanted_speed, Some(1.0));
        assert!(hop.aggressive);
        assert!((10..30).contains(&jump_delay()));
    }
}
