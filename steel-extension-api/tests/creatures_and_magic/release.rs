//! Release through the party menu, reuse the slot, and reject old controls and in-flight balls.
use super::*;
use steel_core::{
    entity::{Entity, SharedEntity},
    inventory::container::Container,
    world::World,
};
use steel_registry::data_components::vanilla_components::CUSTOM_DATA;

const PATH: &str = "creatures:state/party/0";

fn text(player: &Player, field: &str) -> Result<String, String> {
    player
        .data()
        .get(&DataPath::parse(&format!("{PATH}/{field}"))?)
        .and_then(NbtTag::string)
        .map(ToString::to_string)
        .ok_or_else(|| format!("missing creature {field}"))
}

fn click(player: &Player, slot: i16) -> Result<(), String> {
    let container_id = i32::from(player.open_container_id().ok_or("party menu is closed")?);
    player.handle_container_click(SContainerClick {
        container_id,
        state_id: 0,
        slot_num: slot,
        button_num: 0,
        click_type: ClickType::Pickup,
        changed_slots: FxHashMap::default(),
        carried_item: HashedStack::Empty,
    });
    Ok(())
}

async fn launch_ball(
    running: &common::Running,
    world: &Arc<World>,
) -> Result<(String, SharedEntity), String> {
    let player = &running.player;
    common::command(&running.server, player, "party").await?;
    let control = text(player, "control")?;
    let actor = Arc::clone(player);
    let level = Arc::clone(world);
    common::on_tick(&running.server, move || {
        click(&actor, 0)?;
        actor.close_container();
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
            .ok_or("party did not give a current occupied ball")?;
        let ball = inventory.remove_item(slot, 1);
        inventory.set_selected_item(ball);
        drop(inventory);
        actor
            .teleport(DVec3::new(8.5, 65.0, 5.5), 0.0, -90.0)
            .map_err(|error| error.to_string())?;
        actor.handle_use_item(SUseItem {
            hand: InteractionHand::MainHand,
            sequence: 41,
            y_rot: 0.0,
            x_rot: -90.0,
        });
        let projectile = level
            .get_entities_in_aabb(&actor.bounding_box().inflate(2.0))
            .into_iter()
            .find(|entity| entity.entity_type() == &vanilla_entities::SNOWBALL)
            .ok_or("occupied ball did not launch")?;
        Ok((control, projectile))
    })
    .await
}

async fn release_and_recapture(
    running: &common::Running,
    world: &Arc<World>,
    uuid: uuid::Uuid,
) -> Result<(), String> {
    let player = &running.player;
    common::command(&running.server, player, "party").await?;
    let actor = Arc::clone(player);
    let level = Arc::clone(world);
    common::on_tick(&running.server, move || {
        click(&actor, 9)?;
        let released = level
            .get_entity_by_uuid(&uuid)
            .ok_or("release button did not return the mob to the world")?;
        assert!(!released.is_removed());
        assert!(actor.data().get(&DataPath::parse(PATH)?).is_none());
        assert!(instances(&actor).is_empty());
        // A normal native capture can now reuse the freed slot, even for the same mob.
        store(&actor, &released, PATH)?;
        let mut data = actor.data();
        data.set(
            &DataPath::parse(&format!("{PATH}/label"))?,
            NbtTag::String("Recaptured".into()),
        )?;
        data.set(&DataPath::parse(&format!("{PATH}/grade"))?, NbtTag::Long(2))?;
        drop(data);
        actor.close_container();
        Ok(())
    })
    .await
}

pub(super) async fn menu_release(
    running: &common::Running,
    world: &Arc<World>,
) -> Result<(), String> {
    let player = &running.player;
    let uuid = text(player, "uuid")?
        .parse::<uuid::Uuid>()
        .map_err(|error| error.to_string())?;
    let (control, projectile) = launch_ball(running, world).await?;

    release_and_recapture(running, world, uuid).await?;
    common::command(&running.server, player, "party").await?;
    assert_ne!(
        text(player, "control")?,
        control,
        "recapturing the same UUID must create a new control identity"
    );
    assert!(
        !projectile.is_removed(),
        "old ball must still be in flight after the slot is reused"
    );
    let actor = Arc::clone(player);
    common::on_tick(&running.server, move || {
        actor.close_container();
        Ok(())
    })
    .await?;
    common::until(player, "old occupied ball impact", || {
        projectile.is_removed()
    })
    .await?;
    assert!(
        instances(player).is_empty(),
        "old in-flight ball must not deploy the replacement creature"
    );
    assert!(world.get_entity_by_uuid(&uuid).is_none());
    let actor = Arc::clone(player);
    let level = Arc::clone(world);
    common::on_tick(&running.server, move || {
        actor.handle_use_item(SUseItem {
            hand: InteractionHand::MainHand,
            sequence: 42,
            y_rot: 0.0,
            x_rot: -90.0,
        });
        assert!(
            !level
                .get_entities_in_aabb(&actor.bounding_box().inflate(2.0))
                .iter()
                .any(|entity| entity.entity_type() == &vanilla_entities::SNOWBALL),
            "old inventory ball must be rejected before launch"
        );
        Ok(())
    })
    .await?;
    assert!(running.sent_packets.lock().iter().any(|packet| {
        packet
            .encoded_data
            .windows(b"no longer controls".len())
            .any(|bytes| bytes == b"no longer controls")
    }));

    common::command(&running.server, player, "party").await?;
    let actor = Arc::clone(player);
    let level = Arc::clone(world);
    common::on_tick(&running.server, move || {
        let deployed = restore(&actor, &level, PATH, DVec3::new(10.5, 65.0, 5.5))?;
        deployed.kill(&level);
        assert_eq!(
            deployed
                .as_living_entity()
                .ok_or("creature is not living")?
                .get_health(),
            1.0
        );
        click(&actor, 9)?;
        assert!(actor.data().get(&DataPath::parse(PATH)?).is_none());
        let released = level
            .get_entity_by_uuid(&uuid)
            .ok_or("recaptured creature was not released")?;
        click(&actor, 9)?;
        assert!(Arc::ptr_eq(
            &level
                .get_entity_by_uuid(&uuid)
                .ok_or("released mob disappeared")?,
            &released
        ));
        return_all(&actor)?;
        assert!(!released.is_removed());
        actor.close_container();
        Ok(())
    })
    .await
}
