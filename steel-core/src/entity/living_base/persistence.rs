//! Loading the living state already represented by Steel's runtime. Attribute
//! values must be restored before health, and effects must not retrigger start hooks.

use super::{LivingEntityBase, MobEffectInstance};
use crate::entity::{EntityPose, EntityReference, LivingEntity, attribute::AttributeModifier};
use simdnbt::borrow::NbtCompound;
use steel_registry::mob_effect::MobEffectRef;
use steel_registry::{REGISTRY, RegistryExt, attribute::AttributeModifierOperation};
use steel_utils::{BlockPos, Identifier, nbt::NbtNumeric};

fn number(nbt: NbtCompound<'_, '_>, key: &str) -> Option<f32> {
    nbt.get(key).and_then(|v| v.codec_f32())
}
fn integer(nbt: NbtCompound<'_, '_>, key: &str) -> Option<i32> {
    nbt.get(key).and_then(|v| v.codec_i32())
}
fn boolean(nbt: NbtCompound<'_, '_>, key: &str, default: bool) -> bool {
    nbt.get(key).and_then(|v| v.codec_bool()).unwrap_or(default)
}

fn attributes(entity: &dyn LivingEntity, nbt: NbtCompound<'_, '_>) {
    let Some(values) = nbt.list("attributes").and_then(|list| list.compounds()) else {
        return;
    };
    let mut attributes = entity.attributes().lock();
    for value in values {
        let Some(key) = value
            .string("id")
            .and_then(|s| s.to_str().parse::<Identifier>().ok())
        else {
            continue;
        };
        let Some(attribute) = REGISTRY.attributes.by_key(&key) else {
            continue;
        };
        if !attributes.has_attribute(attribute) {
            continue;
        }
        let Some(base) = value.get("base").and_then(|v| v.codec_f64()) else {
            continue;
        };
        attributes.set_base_value(attribute, base);
        let Some(modifiers) = value.list("modifiers").and_then(|list| list.compounds()) else {
            continue;
        };
        for modifier in modifiers {
            let Some(id) = modifier
                .string("id")
                .and_then(|s| s.to_str().parse::<Identifier>().ok())
            else {
                continue;
            };
            let Some(amount) = modifier.get("amount").and_then(|v| v.codec_f64()) else {
                continue;
            };
            let Some(operation) = modifier
                .string("operation")
                .and_then(|s| AttributeModifierOperation::by_name(&s.to_str()))
            else {
                continue;
            };
            attributes.set_modifier(
                attribute,
                AttributeModifier {
                    id,
                    amount,
                    operation,
                },
                true,
            );
        }
    }
}

impl MobEffectInstance {
    fn saved_details(effect: MobEffectRef, nbt: NbtCompound<'_, '_>) -> Self {
        let visible = boolean(nbt, "show_particles", true);
        Self {
            effect,
            duration: integer(nbt, "duration").unwrap_or(0),
            amplifier: i32::from(integer(nbt, "amplifier").unwrap_or(0) as u8),
            ambient: boolean(nbt, "ambient", false),
            visible,
            show_icon: boolean(nbt, "show_icon", visible),
            hidden_effect: nbt
                .compound("hidden_effect")
                .map(|hidden| Box::new(Self::saved_details(effect, hidden))),
        }
    }
    fn from_saved(nbt: NbtCompound<'_, '_>) -> Option<Self> {
        let key: Identifier = nbt.string("id")?.to_str().parse().ok()?;
        Some(Self::saved_details(REGISTRY.mob_effects.by_key(&key)?, nbt))
    }
}

impl LivingEntityBase {
    pub(crate) fn load_persistent_state(
        &self,
        entity: &dyn LivingEntity,
        nbt: NbtCompound<'_, '_>,
    ) {
        self.state.lock().absorption_amount = number(nbt, "AbsorptionAmount").unwrap_or(0.0);
        attributes(entity, nbt);
        {
            let mut effects = self.active_mob_effects.lock();
            effects.clear();
            if let Some(values) = nbt.list("active_effects").and_then(|list| list.compounds()) {
                for value in values {
                    if let Some(effect) = MobEffectInstance::from_saved(value) {
                        effects.insert(effect.effect, effect);
                    }
                }
            }
        }
        self.mark_effects_dirty();
        entity.set_health(number(nbt, "Health").unwrap_or_else(|| entity.get_max_health()));
        {
            let mut state = self.state.lock();
            state.hurt_time = i32::from(integer(nbt, "HurtTime").unwrap_or(0) as i16);
            state.death_time = i32::from(integer(nbt, "DeathTime").unwrap_or(0) as i16);
            state.last_hurt_by_player = EntityReference::read(&nbt, "last_hurt_by_player")
                .map(|reference| reference.uuid());
            state.last_hurt_by_player_memory_time =
                integer(nbt, "last_hurt_by_player_memory_time").unwrap_or(0);
            state.last_hurt_by_mob_uuid =
                EntityReference::read(&nbt, "last_hurt_by_mob").map(|reference| reference.uuid());
            state.last_hurt_by_mob_timestamp = entity
                .tick_count()
                .wrapping_sub(integer(nbt, "ticks_since_last_hurt_by_mob").unwrap_or(0));
            state.current_impulse_context_reset_grace_time =
                integer(nbt, "current_impulse_context_reset_grace_time").unwrap_or(0);
            state.current_impulse_impact_pos = nbt
                .list("current_explosion_impact_pos")
                .and_then(|list| list.doubles())
                .filter(|values| values.len() == 3)
                .map(|v| glam::DVec3::new(v[0], v[1], v[2]));
        }
        entity.set_fall_flying(boolean(nbt, "FallFlying", false));
        if let Some(pos) = nbt.int_array("sleeping_pos").filter(|pos| pos.len() == 3) {
            entity.set_sleeping_pos(BlockPos::new(pos[0], pos[1], pos[2]));
            entity.set_pose(EntityPose::Sleeping);
        } else {
            entity.clear_sleeping_pos();
        }
    }
}
