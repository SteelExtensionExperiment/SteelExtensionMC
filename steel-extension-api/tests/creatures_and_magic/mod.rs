//! Two separately linked native extensions, real client actions and native projectile ticks.
#![cfg(feature = "host")]
#[path = "bats.rs"]
mod bats;
#[path = "../common/mod.rs"]
mod common;
#[path = "hud.rs"]
mod hud;
#[path = "release.rs"]
mod release;
#[path = "scenario.rs"]
mod scenario;
#[path = "sheep.rs"]
mod sheep;

use glam::DVec3;
use rustc_hash::FxHashMap;
use simdnbt::owned::NbtTag;
use std::sync::Arc;
use steel_core::entity::custom_data::CustomData;
use steel_core::entity::{Entity, RemovalReason};
use steel_core::{entity::SharedEntity, world::World};
use steel_core::{
    entity::custom_data::DataPath,
    entity::{ENTITIES, LivingEntity, damage::DamageSource, next_entity_id},
    player::Player,
    server::Server,
};
use steel_extension_api::stabby::{abi::IDeterminantProvider, string::String as ApiString};
use steel_extension_api::{
    Server as ApiServer, World as ApiWorld,
    entities::{EntityId, EntityPolicy},
    players::Uuid as ApiUuid,
};
use steel_protocol::packets::game::{ClickType, HashedStack, SContainerClick, SUseItem};
use steel_registry::{
    REGISTRY, RegistryExt, item_stack::ItemStack, vanilla_blocks, vanilla_damage_types,
    vanilla_entities, vanilla_mob_effects,
};
use steel_utils::{
    BlockPos, Identifier,
    types::{InteractionHand, UpdateFlags},
};

fn number(player: &Player, path: &str) -> Result<f64, String> {
    let path = DataPath::parse(path)?;
    player
        .data()
        .get(&path)
        .and_then(CustomData::number)
        .ok_or("missing numeric player data".into())
}

async fn use_item(server: &Server, player: &Arc<Player>, item: &str) -> Result<(), String> {
    let count = if item == "magic:wand" { 1 } else { 4 };
    let item = REGISTRY
        .items
        .by_key(&item.parse::<Identifier>().map_err(str::to_owned)?)
        .ok_or("missing example item")?;
    let player = Arc::clone(player);
    common::on_tick(server, move || {
        player
            .inventory
            .lock()
            .set_selected_item(ItemStack::with_count(item, count));
        player.handle_use_item(SUseItem {
            hand: InteractionHand::MainHand,
            sequence: 1,
            y_rot: 0.0,
            x_rot: 12.0,
        });
        Ok(())
    })
    .await
}

async fn spell(server: &Server, player: &Arc<Player>, index: i16) -> Result<(), String> {
    use_item(server, player, "magic:wand").await?;
    common::until(player, "wand menu", || player.open_container_id().is_some()).await?;
    let container_id = i32::from(player.open_container_id().ok_or("wand menu closed")?);
    let player = Arc::clone(player);
    common::on_tick(server, move || {
        player.handle_container_click(SContainerClick {
            container_id,
            state_id: 0,
            slot_num: index,
            button_num: 0,
            click_type: ClickType::Pickup,
            changed_slots: FxHashMap::default(),
            carried_item: HashedStack::Empty,
        });
        Ok(())
    })
    .await
}

#[test]
#[ignore = "requires debug builds of extension-example/creatures and extension-example/magic"]
fn independent_examples_capture_cast_and_protect_on_a_vanilla_connection() -> Result<(), String> {
    let root = tempfile::tempdir().map_err(|e| e.to_string())?;
    let loaded = common::load_extensions(
        &[
            ("creatures", "base_chance=[1.0,1.0,1.0]"),
            (
                "magic",
                "regeneration=0.0\nprotection_multiplier=0.0\nterrain_ticks=100",
            ),
        ],
        root.path(),
    )?;
    let initialized = loaded.initialized;
    let fixture = common::Fixture::new()?;
    let runtime = Arc::clone(&fixture.runtime);
    runtime.block_on(async {
        let uuid = uuid::Uuid::new_v4();
        let running = common::start(
            &fixture.world,
            (loaded.extensions, loaded.commands),
            common::grants(uuid, &[])?,
            uuid,
            "TrainerMage",
        )
        .await?;
        let result = async {
            scenario::bootstrap(&running).await?;
            sheep::battle(&running, &fixture.world, sheep::AimInput::Unchanged).await?;
            sheep::battle(&running, &fixture.world, sheep::AimInput::TurnAndUse).await?;
            sheep::battle(&running, &fixture.world, sheep::AimInput::TurnAndInteract).await?;
            sheep::weakened_battle(&running, &fixture.world).await?;
            scenario::capture(&running, &fixture.world).await?;
            scenario::deploy_and_recall(&running, &fixture.world).await?;
            scenario::attack_spells(&running, &fixture.world).await?;
            scenario::terrain(&running, &fixture.world).await?;
            scenario::dependency_data(&running).await?;
            hud::shared_scoreboard(&running, &fixture.world).await?;
            release::menu_release(&running, &fixture.world).await?;
            bats::capture_and_battle(&running, &fixture.world).await?;
            Ok::<_, String>(())
        }
        .await;
        common::stop(running, &initialized, &fixture.world).await?;
        result
    })
}

fn api_result<T: IDeterminantProvider<ApiString>>(
    result: steel_extension_api::ApiResult<T>,
) -> Result<T, String> {
    result
        .match_owned(Ok, Err)
        .map_err(|error| error.to_string())
}
fn sdk_player(player: &Player) -> steel_extension_api::PlayerId {
    ApiServer::from_host(&player.server())
        .players()
        .into_iter()
        .find(|resident| resident.id.uuid().bytes == *player.uuid().as_bytes())
        .expect("test player should be resident")
        .id
}
fn instances(player: &Player) -> Vec<(String, SharedEntity)> {
    let server = player.server();
    api_result(ApiServer::from_host(&server).stored_entity_instances(sdk_player(player)))
        .expect("stored instances should be readable")
        .into_iter()
        .map(|entry| {
            let entity = player
                .get_world()
                .get_entity_by_id(entry.entity.id)
                .expect("stored instance should exist in world");
            assert_eq!(entity.uuid().as_bytes(), &entry.entity.uuid.bytes);
            (entry.path.to_string(), entity)
        })
        .collect()
}
fn store(player: &Player, entity: &SharedEntity, path: &str) -> Result<(), String> {
    let world = player.get_world();
    api_result(ApiWorld::from_host(&world).store_entity(
        sdk_player(player),
        EntityId {
            id: entity.id(),
            uuid: ApiUuid {
                bytes: *entity.uuid().as_bytes(),
            },
        },
        path.into(),
        EntityPolicy {
            minimum_health: 1.0,
            drop_loot: false,
        },
    ))
}
fn restore(
    player: &Player,
    world: &Arc<World>,
    path: &str,
    pos: DVec3,
) -> Result<SharedEntity, String> {
    assert!(Arc::ptr_eq(&player.get_world(), world));
    let server = player.server();
    let info = api_result(ApiServer::from_host(&server).restore_entity(
        sdk_player(player),
        path.into(),
        steel_extension_api::Position {
            x: pos.x,
            y: pos.y,
            z: pos.z,
        },
        EntityPolicy {
            minimum_health: 1.0,
            drop_loot: false,
        },
    ))?;
    world
        .get_entity_by_id(info.id)
        .ok_or("restored entity missing".into())
}
fn return_all(player: &Player) -> Result<(), String> {
    let server = player.server();
    for (path, _) in instances(player) {
        api_result(
            ApiServer::from_host(&server).return_entity(sdk_player(player), path.as_str().into()),
        )?;
    }
    Ok(())
}
