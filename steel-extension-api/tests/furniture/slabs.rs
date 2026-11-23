use std::{error::Error, f32::consts::PI};

use glam::{DVec3, Quat, Vec3};
use serde::Deserialize;
use steel_core::{entity::Entity, physics::WorldCollisionProvider};
use steel_protocol::packets::game::SUseItemOn;
use steel_registry::{
    REGISTRY, RegistryExt,
    blocks::{
        block_state_ext::BlockStateExt,
        properties::BlockStateProperties,
        shapes::{BooleanOp, join_is_not_empty},
    },
    client_content::{BlockDisplayTransform, client_block_state},
    data_components::vanilla_components::ITEM_MODEL,
    item_stack::ItemStack,
    items::item::BlockHitResult,
    packets::play::C_BLOCK_UPDATE,
    vanilla_blocks, vanilla_items,
};
use steel_utils::{
    BlockPos, BlockStateId, Direction, Identifier, WorldAabb,
    serial::ReadFrom,
    types::{InteractionHand, UpdateFlags},
};

use super::{
    fixture::{Fixture, packet_payload},
    helpers,
};

pub(super) fn check_geometry() -> Result<(), Box<dyn Error>> {
    let block = helpers::block("cut_copper_vertical_slab")?;
    let model = REGISTRY
        .client_content
        .block(block)
        .ok_or("missing slab model")?;
    let packed_model = REGISTRY
        .client_content
        .resource_pack()
        .files()
        .find(|(path, _)| *path == "assets/furniture/items/cut_copper_vertical_slab.json")
        .ok_or("missing packed slab geometry")?;
    let packed_model: serde_json::Value = serde_json::from_slice(packed_model.1)?;
    let packed_model = &packed_model["model"];
    assert_eq!(packed_model["property"], "minecraft:display_context");
    let display_model = &packed_model["cases"][0];
    assert_eq!(display_model["when"], "none");
    assert_eq!(
        display_model["model"]["model"],
        "minecraft:block/cut_copper_slab"
    );
    assert_eq!(
        packed_model["fallback"]["model"],
        display_model["model"]["model"]
    );
    let fallback = vanilla_blocks::CUT_COPPER_SLAB
        .default_state()
        .get_static_outline_shape();
    let [fallback] = fallback.boxes() else {
        return Err("expected a single vanilla slab cuboid".into());
    };
    let fallback_bounds = (
        Vec3::new(
            fallback.min_x() as f32,
            fallback.min_y() as f32,
            fallback.min_z() as f32,
        ),
        Vec3::new(
            fallback.max_x() as f32,
            fallback.max_y() as f32,
            fallback.max_z() as f32,
        ),
    );
    // The inventory model turns the same horizontal slab into a vertical half.
    let item_transform = &packed_model["fallback"]["transformation"];
    let rotation = Quat::from_array(<[f32; 4]>::deserialize(&item_transform["left_rotation"])?);
    let translation = Vec3::from_array(<[f32; 3]>::deserialize(&item_transform["translation"])?);
    let (min, max) = bounds(fallback_bounds, |point| translation + rotation * point);
    assert!(min.abs_diff_eq(Vec3::new(0.0, 0.0, 0.5), 1.0e-5));
    assert!(max.abs_diff_eq(Vec3::ONE, 1.0e-5));
    for direction in [
        Direction::North,
        Direction::East,
        Direction::South,
        Direction::West,
    ] {
        let state = block
            .default_state()
            .set_value(&BlockStateProperties::HORIZONTAL_FACING, direction);
        let client = client_block_state(state);
        let server_shape = state.get_static_collision_shape();
        let client_shape = client.get_static_collision_shape();
        assert!(
            !join_is_not_empty(server_shape, client_shape, BooleanOp::NotSame),
            "{direction:?}: server collision {:?} differs from {} client collision {:?}",
            server_shape.boxes(),
            client.get_block().key,
            client_shape.boxes(),
        );
        let expected = REGISTRY
            .blocks
            .state_id_from_block_defaulted_properties(
                &vanilla_blocks::OAK_SHELF,
                [("facing", direction.opposite().as_str())],
            )
            .ok_or("missing dry, unconnected shelf state")?;
        assert_eq!(client, expected);
        assert!(!state.has_block_entity());
        assert!(!join_is_not_empty(
            state.get_static_outline_shape(),
            client.get_static_outline_shape(),
            BooleanOp::NotSame,
        ));
        let part = model
            .displays_for(state)
            .first()
            .ok_or("missing slab display")?;
        assert!(part.pack_transform.is_none());
        let transform = BlockDisplayTransform {
            translation: part.translation,
            scale: part.scale,
            rotation: part.rotation,
        };
        let (min, max) = displayed_bounds(transform, fallback_bounds);
        let size = max - min;
        let thickness = size.x.min(size.z);
        assert!(
            (0.5..0.505).contains(&thickness),
            "{direction:?} mesh should look half a block thick, got {thickness}"
        );
        for carrier in client_shape.boxes() {
            let carrier_min = Vec3::new(
                carrier.min_x() as f32,
                carrier.min_y() as f32,
                carrier.min_z() as f32,
            );
            let carrier_max = Vec3::new(
                carrier.max_x() as f32,
                carrier.max_y() as f32,
                carrier.max_z() as f32,
            );
            assert!(
                min.cmplt(carrier_min).all() && max.cmpgt(carrier_max).all(),
                "{direction:?} mesh {min:?}..{max:?} exposes carrier {carrier_min:?}..{carrier_max:?}"
            );
        }
    }
    Ok(())
}

fn displayed_bounds(transform: BlockDisplayTransform, mesh: (Vec3, Vec3)) -> (Vec3, Vec3) {
    let renderer_rotation = Quat::from_rotation_y(PI);
    bounds(mesh, |point| {
        // The item renderer centers the mesh and turns it 180 degrees
        // before the display entity's scale and rotation apply.
        let point = renderer_rotation * (point - Vec3::splat(0.5));
        transform.translation + transform.rotation * (transform.scale * point)
    })
}

fn bounds((from, to): (Vec3, Vec3), transform: impl Fn(Vec3) -> Vec3) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for x in [from.x, to.x] {
        for y in [from.y, to.y] {
            for z in [from.z, to.z] {
                let point = transform(Vec3::new(x, y, z));
                min = min.min(point);
                max = max.max(point);
            }
        }
    }
    (min, max)
}

pub(super) fn check(fixture: &Fixture) -> Result<(), Box<dyn Error>> {
    let mut materials = 0;
    for (_, source) in REGISTRY.blocks.iter() {
        if !REGISTRY.client_content.is_vanilla_block(source)
            || REGISTRY
                .blocks
                .state_id_from_block_defaulted_properties(
                    source,
                    [("type", "double"), ("waterlogged", "false")],
                )
                .is_none()
        {
            continue;
        }
        materials += 1;
        let name = source
            .key
            .path
            .strip_suffix("_slab")
            .ok_or("unexpected slab path")?;
        let key = Identifier::new("furniture", format!("{name}_vertical_slab"));
        let block = REGISTRY
            .blocks
            .by_key(&key)
            .ok_or("missing material variant")?;
        let model = REGISTRY
            .client_content
            .block(block)
            .ok_or("missing model")?;
        assert_eq!(model.material, Some(source));
        let part = model.displays.first().ok_or("missing slab display")?;
        let item = REGISTRY.items.by_block(block);
        assert_eq!(item.components.get(ITEM_MODEL), Some(key));
        assert_eq!(
            part.item.get(ITEM_MODEL),
            item.components.get(ITEM_MODEL).as_ref()
        );
    }
    assert!(materials > 0);
    let pos = BlockPos::new(8, 65, 8);
    for name in [
        "spruce_vertical_slab",
        "stone_vertical_slab",
        "cut_copper_vertical_slab",
        "vertical_slab",
    ] {
        fixture
            .player
            .try_set_position(DVec3::new(6.5, 65.0, 8.5))?;
        helpers::place(fixture, name, pos)?;
        let state = fixture.world.get_block_state(pos);
        let collision = WorldCollisionProvider::new(&fixture.world);
        assert!(collision.has_entity_context_collision(
            WorldAabb::new(8.1, 65.1, 8.9, 8.9, 65.9, 8.95),
            65.1,
            false
        ));
        assert!(collision.has_entity_context_collision(
            WorldAabb::new(8.1, 65.1, 8.72, 8.9, 65.2, 8.78),
            65.1,
            false
        ));
        assert!(!collision.has_entity_context_collision(
            WorldAabb::new(8.1, 65.4, 8.72, 8.9, 65.6, 8.78),
            65.4,
            false
        ));
        helpers::interact(fixture, pos);
        assert_eq!(fixture.world.get_block_state(pos), state);
        let loaded = helpers::saved(fixture)?;
        assert_eq!(loaded.chunk.get_block_state(pos), state);
        fixture
            .player
            .inventory
            .lock()
            .set_selected_item(ItemStack::new(&vanilla_items::NETHERITE_PICKAXE));
        helpers::mine(fixture, pos);
        assert!(fixture.world.get_block_state(pos).is_air());
    }
    Ok(())
}

pub(super) fn check_shelf_face_click(fixture: &Fixture) -> Result<(), Box<dyn Error>> {
    let pos = BlockPos::new(8, 65, 8);
    let state = helpers::block("spruce_vertical_slab")?.default_state();
    let carrier = client_block_state(state);
    let face = carrier.get_value(&BlockStateProperties::HORIZONTAL_FACING);
    let neighbor = face.relative(pos);
    let _runtime = fixture.runtime.enter();
    fixture.world.set_block(pos, state, UpdateFlags::UPDATE_ALL);
    fixture
        .player
        .try_set_position(DVec3::new(8.5, 65.0, 10.5))?;
    assert!(!fixture.player.is_awaiting_teleport());
    fixture
        .player
        .inventory
        .lock()
        .set_selected_item(ItemStack::new(&vanilla_items::STONE));
    fixture.packets.lock().clear();
    // The vanilla shelf may consume this click for prediction. Steel still
    // receives the use-on packet and places against the actual slab.
    fixture.player.handle_use_item_on(SUseItemOn {
        hand: InteractionHand::MainHand,
        block_hit: BlockHitResult {
            location: DVec3::new(
                f64::from(pos.x()) + 0.5,
                f64::from(pos.y()) + 0.5,
                f64::from(pos.z())
                    + state
                        .get_static_outline_shape()
                        .max(steel_utils::axis::Axis::Z),
            ),
            direction: face,
            block_pos: pos,
            miss: false,
            inside: false,
            world_border_hit: false,
        },
        sequence: 1,
    });
    let stone = vanilla_blocks::STONE.default_state();
    assert_eq!(fixture.world.get_block_state(pos), state);
    assert_eq!(fixture.world.get_block_state(neighbor), stone);
    let updates = fixture
        .packets
        .lock()
        .iter()
        .filter_map(|packet| packet_payload(packet, C_BLOCK_UPDATE))
        .map(|mut payload| {
            Ok((
                BlockPos::read(&mut payload)?,
                BlockStateId::read(&mut payload)?,
            ))
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    assert!(updates.contains(&(pos, carrier)));
    assert!(updates.contains(&(neighbor, stone)));
    assert!(fixture.world.destroy_block(neighbor, false));
    assert!(fixture.world.destroy_block(pos, false));
    fixture
        .player
        .try_set_position(DVec3::new(6.5, 65.0, 8.5))?;
    Ok(())
}
