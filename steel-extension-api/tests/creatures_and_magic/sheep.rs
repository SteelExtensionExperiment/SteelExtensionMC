//! Capture, mint a party ball, throw it, and observe real sheep combat with AI enabled.
use super::*;
use steel_core::{
    entity::{Entity, SharedEntity},
    inventory::container::Container,
    world::World,
};
use steel_protocol::packets::game::SInteract;
use steel_registry::data_components::vanilla_components::CUSTOM_DATA;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AimInput {
    Unchanged,
    TurnAndUse,
    TurnAndInteract,
}

async fn prepare(
    running: &common::Running,
    world: &Arc<World>,
    health: Option<f32>,
) -> Result<(), String> {
    let player = &running.player;
    let server = &running.server;
    let actor = Arc::clone(player);
    let level = Arc::clone(world);
    common::on_tick(server, move || {
        let sheep = ENTITIES
            .create(
                &vanilla_entities::SHEEP,
                next_entity_id(),
                DVec3::new(10.5, 65.0, 8.5),
                Arc::downgrade(&level),
            )
            .ok_or("sheep factory missing")?;
        if let Some(health) = health {
            sheep
                .as_living_entity()
                .ok_or("sheep is not living")?
                .set_health(health);
        }
        level
            .try_add_entity(Arc::clone(&sheep))
            .map_err(|e| e.to_string())?;
        store(&actor, &sheep, "creatures:state/party/0")?;
        let mut data = actor.data();
        data.set(
            &DataPath::parse("creatures:state/party/0/grade")?,
            NbtTag::Long(2),
        )?;
        data.set(
            &DataPath::parse("creatures:state/party/0/label")?,
            NbtTag::String("sheep".into()),
        )
    })
    .await?;
    common::command(server, player, "party").await?;
    common::until(player, "sheep party menu", || {
        player.open_container_id().is_some()
    })
    .await?;
    let menu = i32::from(player.open_container_id().ok_or("party menu closed")?);
    let actor = Arc::clone(player);
    common::on_tick(server, move || {
        actor.handle_container_click(SContainerClick {
            container_id: menu,
            state_id: 0,
            slot_num: 0,
            button_num: 0,
            click_type: ClickType::Pickup,
            changed_slots: FxHashMap::default(),
            carried_item: HashedStack::Empty,
        });
        actor.close_container();
        Ok(())
    })
    .await?;
    Ok(())
}

async fn launch(
    running: &common::Running,
    world: &Arc<World>,
    input: AimInput,
) -> Result<SharedEntity, String> {
    let player = &running.player;
    let server = &running.server;
    let entity_click = input == AimInput::TurnAndInteract;
    let changed_aim = input != AimInput::Unchanged;
    let target = ENTITIES
        .create(
            &vanilla_entities::SHEEP,
            next_entity_id(),
            DVec3::new(8.5, 65.0, if entity_click { 8.0 } else { 10.0 }),
            Arc::downgrade(world),
        )
        .ok_or("sheep factory missing")?;
    let wild = Arc::clone(&target);
    let target_id = target.id();
    let actor = Arc::clone(player);
    let level = Arc::clone(world);
    common::on_tick(server, move || {
        level.try_add_entity(wild).map_err(|e| e.to_string())?;
        // A wrong previous yaw hits this backdrop and deploys without an opponent.
        for z in 4..=7 {
            for y in 65..=68 {
                level.set_block(
                    BlockPos::new(5, y, z),
                    vanilla_blocks::STONE.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
            }
        }
        let identity = actor
            .data()
            .get(&DataPath::parse("creatures:state/party/0/control")?)
            .and_then(NbtTag::string)
            .map(ToString::to_string)
            .ok_or("party control missing")?;
        let mut inventory = actor.inventory.lock();
        let slot = inventory
            .get_items()
            .iter()
            .position(|stack| {
                stack
                    .get(CUSTOM_DATA)
                    .and_then(|data| data.as_compound().compound("steel_extensions"))
                    .is_some_and(|entries| {
                        entries
                            .string("creatures:control")
                            .is_some_and(|value| value.to_string() == identity)
                    })
            })
            .ok_or("party menu did not give the sheep ball")?;
        let ball = inventory.remove_item(slot, 1);
        inventory.set_selected_item(ball);
        drop(inventory);
        actor
            .teleport(
                DVec3::new(8.5, 65.0, 5.5),
                if changed_aim { 90.0 } else { 0.0 },
                12.0,
            )
            .map_err(|e| e.to_string())?;
        if entity_click {
            actor.handle_interact(SInteract {
                entity_id: target_id,
                hand: InteractionHand::MainHand,
                location: DVec3::new(0.0, 0.75, 0.0),
                using_secondary_action: false,
            });
        }
        actor.handle_use_item(SUseItem {
            hand: InteractionHand::MainHand,
            sequence: 11,
            y_rot: 0.0,
            x_rot: 12.0,
        });
        Ok(())
    })
    .await?;
    Ok(target)
}

pub(super) async fn battle(
    running: &common::Running,
    world: &Arc<World>,
    input: AimInput,
) -> Result<(), String> {
    let player = &running.player;
    prepare(running, world, None).await?;
    let target = launch(running, world, input).await?;
    let started = world.game_time();
    common::until(player, "sheep deployment", || {
        !instances(player).is_empty() || world.game_time() - started > 30
    })
    .await?;
    let deployments = instances(player);
    let ally = deployments.first().ok_or("sheep ball did not deploy")?;
    common::until(player, "sheep damaging its opponent", || {
        target
            .as_living_entity()
            .is_some_and(|living| living.get_health() < living.get_max_health())
            || world.game_time() - started > 120
    })
    .await?;
    assert!(
        target
            .as_living_entity()
            .is_some_and(|living| living.get_health() < living.get_max_health()),
        "sheep did not attack: input={input:?}, player_rotation={:?}, ally_position={:?}, target_position={:?}",
        player.rotation(),
        ally.1.position(),
        target.position()
    );
    common::until(player, "nonlethal sheep battle completion", || {
        instances(player).is_empty() || world.game_time() - started > 240
    })
    .await?;
    assert_eq!(instances(player).len(), 0);
    assert!(
        target
            .as_living_entity()
            .is_some_and(|living| living.get_health() >= 1.0)
    );
    cleanup(running, world, target).await
}

pub(super) async fn weakened_battle(
    running: &common::Running,
    world: &Arc<World>,
) -> Result<(), String> {
    let player = &running.player;
    prepare(running, world, Some(1.0)).await?;
    let message_start = running.sent_packets.lock().len();
    let target = launch(running, world, AimInput::TurnAndUse).await?;
    let started = world.game_time();
    let contains = |text: &[u8]| {
        running
            .sent_packets
            .lock()
            .iter()
            .skip(message_start)
            .any(|packet| {
                packet
                    .encoded_data
                    .windows(text.len())
                    .any(|bytes| bytes == text)
            })
    };
    common::until(player, "weak sheep healing feedback", || {
        contains(b"needs healing") || world.game_time() - started > 40
    })
    .await?;
    assert!(
        contains(b"needs healing"),
        "a weak creature's rejected battle must explain recovery in client chat"
    );
    assert!(
        !contains(b"Battle ended"),
        "an ineligible battle must not report completion"
    );
    assert!(instances(player).is_empty());
    assert_eq!(
        number(player, "creatures:state/party/0/health/current")?,
        1.0
    );
    let living = target.as_living_entity().ok_or("sheep is not living")?;
    assert_eq!(living.get_health(), living.get_max_health());
    cleanup(running, world, target).await
}

async fn cleanup(
    running: &common::Running,
    world: &Arc<World>,
    target: SharedEntity,
) -> Result<(), String> {
    let actor = Arc::clone(&running.player);
    let level = Arc::clone(world);
    common::on_tick(&running.server, move || {
        target.set_removed(RemovalReason::Discarded);
        actor
            .data()
            .remove(&DataPath::parse("creatures:state/party/0")?);
        for z in 4..=7 {
            for y in 65..=68 {
                level.set_block(
                    BlockPos::new(5, y, z),
                    vanilla_blocks::AIR.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
            }
        }
        Ok(())
    })
    .await?;
    Ok(())
}
