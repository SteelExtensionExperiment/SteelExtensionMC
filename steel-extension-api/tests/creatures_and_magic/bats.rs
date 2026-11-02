//! Actual ball impacts capture a flying bat and deploy it for a nonlethal aerial battle.
use super::*;
use steel_core::{
    entity::{SharedEntity, entities::BatEntity},
    inventory::container::Container,
    world::World,
};
use steel_protocol::packets::game::SInteract;
use steel_registry::data_components::vanilla_components::CUSTOM_DATA;
use steel_utils::Downcast as _;

const PATH: &str = "creatures:state/party/0";

fn flying_bat(world: &Arc<World>) -> Result<SharedEntity, String> {
    let bat = ENTITIES
        .create(
            &vanilla_entities::BAT,
            next_entity_id(),
            DVec3::new(8.5, 66.0, 8.0),
            Arc::downgrade(world),
        )
        .ok_or("bat factory missing")?;
    bat.downcast_ref::<BatEntity>()
        .ok_or("factory did not create a bat")?
        .set_resting(false);
    world
        .try_add_entity(Arc::clone(&bat))
        .map_err(|error| error.to_string())?;
    Ok(bat)
}

fn throw_at(player: &Arc<Player>, target: &SharedEntity) {
    player.handle_interact(SInteract {
        entity_id: target.id(),
        hand: InteractionHand::MainHand,
        location: DVec3::new(0.0, 0.45, 0.0),
        using_secondary_action: false,
    });
    player.handle_use_item(SUseItem {
        hand: InteractionHand::MainHand,
        sequence: 61,
        y_rot: 0.0,
        x_rot: 0.0,
    });
}

async fn capture(running: &common::Running, world: &Arc<World>) -> Result<SharedEntity, String> {
    let player = &running.player;
    let actor = Arc::clone(player);
    let level = Arc::clone(world);
    let bat = common::on_tick(&running.server, move || {
        assert!(actor.data().get(&DataPath::parse(PATH)?).is_none());
        actor
            .teleport(DVec3::new(8.5, 65.0, 5.5), 0.0, 0.0)
            .map_err(|error| error.to_string())?;
        let item = REGISTRY
            .items
            .by_key(&Identifier::new_static("creatures", "hyper_ball"))
            .ok_or("hyper ball not registered")?;
        actor
            .inventory
            .lock()
            .set_selected_item(ItemStack::with_count(item, 4));
        let bat = flying_bat(&level)?;
        throw_at(&actor, &bat);
        Ok(bat)
    })
    .await?;
    common::until(player, "flying bat capture", || bat.is_removed()).await?;
    let saved_uuid = player
        .data()
        .get(&DataPath::parse(&format!("{PATH}/uuid"))?)
        .and_then(NbtTag::string)
        .map(ToString::to_string)
        .ok_or("bat capture did not populate the roster")?;
    assert_eq!(saved_uuid, bat.uuid().to_string());
    Ok(bat)
}

pub(super) async fn capture_and_battle(
    running: &common::Running,
    world: &Arc<World>,
) -> Result<(), String> {
    let player = &running.player;
    let bat = capture(running, world).await?;

    common::command(&running.server, player, "party").await?;
    let actor = Arc::clone(player);
    let level = Arc::clone(world);
    let wild = common::on_tick(&running.server, move || {
        let menu = i32::from(actor.open_container_id().ok_or("party menu closed")?);
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
        let control = actor
            .data()
            .get(&DataPath::parse(&format!("{PATH}/control"))?)
            .and_then(NbtTag::string)
            .map(ToString::to_string)
            .ok_or("captured bat has no controls")?;
        let mut inventory = actor.inventory.lock();
        let slot = inventory
            .get_items()
            .iter()
            .position(|stack| {
                stack
                    .get(CUSTOM_DATA)
                    .and_then(|data| data.as_compound().compound("steel_extensions"))
                    .and_then(|data| data.string("creatures:control"))
                    .is_some_and(|value| value.to_string() == control)
            })
            .ok_or("party did not give a bat ball")?;
        let ball = inventory.remove_item(slot, 1);
        inventory.set_selected_item(ball);
        drop(inventory);
        let wild = flying_bat(&level)?;
        throw_at(&actor, &wild);
        Ok(wild)
    })
    .await?;
    common::until(player, "bat battle deployment", || {
        !instances(player).is_empty()
    })
    .await?;
    assert_eq!(instances(player)[0].1.uuid(), bat.uuid());
    common::until(player, "bat battle damage", || {
        wild.as_living_entity()
            .is_some_and(|living| living.get_health() < living.get_max_health())
    })
    .await?;
    common::until(player, "bat battle recall", || instances(player).is_empty()).await?;
    assert!(
        wild.as_living_entity()
            .ok_or("opponent is not living")?
            .get_health()
            >= 1.0
    );
    assert!(number(player, &format!("{PATH}/health/current"))? >= 1.0);
    assert_eq!(player.hud_conflicts(), Vec::<String>::new());
    let actor = Arc::clone(player);
    common::on_tick(&running.server, move || {
        wild.set_removed(RemovalReason::Discarded);
        actor.data().remove(&DataPath::parse(PATH)?);
        Ok(())
    })
    .await
}
