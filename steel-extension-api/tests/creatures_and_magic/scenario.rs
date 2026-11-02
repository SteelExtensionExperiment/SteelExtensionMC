//! Native gameplay phases share one server, but verify separate domain behavior.
use super::*;
use steel_core::inventory::container::Container;
use steel_core::world::World;
use steel_protocol::packets::game::SInteract;
use steel_registry::data_components::vanilla_components::CUSTOM_DATA;

async fn ball_ready(player: &Arc<Player>) -> Result<(), String> {
    let path = DataPath::parse("creatures:state/throw_after")?;
    common::until(player, "ball use interval", || {
        let now = player.get_world().game_time();
        player
            .data()
            .get(&path)
            .and_then(NbtTag::long)
            .is_some_and(|ready| ready <= now)
    })
    .await
}

pub(super) async fn deploy_and_recall(
    running: &common::Running,
    world: &Arc<World>,
) -> Result<(), String> {
    let player = &running.player;
    let server = &running.server;
    ball_ready(player).await?;
    let target = ENTITIES
        .create(
            &vanilla_entities::COW,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 9.5),
            Arc::downgrade(world),
        )
        .ok_or("cow factory missing")?;
    let cow = Arc::clone(&target);
    let level = Arc::clone(world);
    let actor = Arc::clone(player);
    common::on_tick(server, move || {
        cow.as_mob().ok_or("cow should be a mob")?.set_no_ai(true);
        level.try_add_entity(cow).map_err(|e| e.to_string())?;
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
            .ok_or("capture did not give an occupied ball")?;
        let control = inventory.remove_item(slot, 1);
        inventory.set_selected_item(control);
        drop(inventory);
        actor.handle_use_item(SUseItem {
            hand: InteractionHand::MainHand,
            sequence: 2,
            y_rot: 0.0,
            x_rot: 12.0,
        });
        Ok(())
    })
    .await?;
    common::until(player, "occupied-ball deployment", || {
        !instances(player).is_empty()
    })
    .await?;
    let deployments = instances(player);
    assert_eq!(deployments.len(), 1);
    assert_eq!(deployments[0].0, "creatures:state/party/0");
    assert!(
        deployments[0]
            .1
            .as_mob()
            .is_some_and(|mob| mob.mob_base().ai_controller().is_active()),
        "starting a battle must retain its active controller"
    );
    ball_ready(player).await?;
    let actor = Arc::clone(player);
    common::on_tick(server, move || {
        actor.handle_use_item(SUseItem {
            hand: InteractionHand::MainHand,
            sequence: 3,
            y_rot: 0.0,
            x_rot: 12.0,
        });
        assert_eq!(instances(&actor).len(), 0);
        Ok(())
    })
    .await?;
    assert!(deployments[0].1.is_removed());
    let cow = Arc::clone(&target);
    common::on_tick(server, move || {
        cow.set_removed(RemovalReason::Discarded);
        Ok(())
    })
    .await?;
    Ok(())
}

pub(super) async fn bootstrap(running: &common::Running) -> Result<(), String> {
    let player = &running.player;
    let server = &running.server;
    common::until(player, "mana and roster HUD", || {
        number(player, "magic:state/mana/current") == Ok(100.0)
            && running.sent_packets.lock().iter().any(|packet| {
                packet
                    .encoded_data
                    .windows(9)
                    .any(|bytes| bytes == b"Your team")
            })
            && running
                .sent_packets
                .lock()
                .iter()
                .any(|packet| packet.encoded_data.windows(4).any(|bytes| bytes == b"Mana"))
    })
    .await?;
    let actor = Arc::clone(player);
    common::on_tick(server, move || {
        actor
            .teleport(DVec3::new(8.5, 65.0, 5.5), 0.0, 12.0)
            .map_err(|e| e.to_string())
    })
    .await?;
    spell(server, player, 7).await?;
    assert_eq!(number(player, "magic:state/mana/current")?, 75.0);
    let actor = Arc::clone(player);
    common::on_tick(server, move || {
        let before = actor.get_health();
        assert!(!actor.hurt(
            &actor.get_world(),
            &DamageSource::environment(&vanilla_damage_types::THROWN),
            4.0
        ));
        assert_eq!(actor.get_health(), before);
        Ok(())
    })
    .await?;
    spell(server, player, 8).await?;
    assert_eq!(number(player, "magic:state/mana/current")?, 45.0);
    let actor = Arc::clone(player);
    common::on_tick(server, move || {
        let before = actor.get_health();
        assert!(!actor.hurt(
            &actor.get_world(),
            &DamageSource::environment(&vanilla_damage_types::MOB_ATTACK),
            4.0
        ));
        assert_eq!(actor.get_health(), before);
        assert!(actor.hurt(
            &actor.get_world(),
            &DamageSource::environment(&vanilla_damage_types::FALL),
            3.0
        ));
        assert!(
            actor.get_health() < before,
            "melee shield must not guess that all nonprojectile damage is physical"
        );
        Ok(())
    })
    .await?;
    Ok(())
}

pub(super) async fn capture(running: &common::Running, world: &Arc<World>) -> Result<(), String> {
    let player = &running.player;
    let server = &running.server;
    let world = Arc::clone(world);
    let cow = ENTITIES
        .create(
            &vanilla_entities::COW,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.0),
            Arc::downgrade(&world),
        )
        .ok_or("cow factory missing")?;
    let target = Arc::clone(&cow);
    common::on_tick(server, move || {
        target
            .as_mob()
            .ok_or("cow should be a mob")?
            .set_no_ai(true);
        world.try_add_entity(target).map_err(|e| e.to_string())
    })
    .await?;
    spell(server, player, 1).await?;
    common::until(player, "native ice impact", || {
        cow.as_living_entity().is_some_and(|living| {
            living.get_health() < 10.0 && living.has_mob_effect(vanilla_mob_effects::SLOWNESS)
        })
    })
    .await?;
    assert_eq!(number(player, "magic:state/mana/current")?, 35.0);
    let ball = REGISTRY
        .items
        .by_key(&Identifier::new_static("creatures", "hyper_ball"))
        .ok_or("hyper ball not registered")?;
    let actor = Arc::clone(player);
    let target = Arc::clone(&cow);
    common::on_tick(server, move || {
        actor
            .inventory
            .lock()
            .set_selected_item(ItemStack::with_count(ball, 4));
        actor.handle_interact(SInteract {
            entity_id: target.id(),
            hand: InteractionHand::MainHand,
            location: DVec3::new(0.0, 0.8, 0.0),
            using_secondary_action: false,
        });
        assert_eq!(
            actor.inventory.lock().get_selected_item().count(),
            3,
            "entity interaction must launch the ball"
        );
        actor.handle_use_item(SUseItem {
            hand: InteractionHand::MainHand,
            sequence: 1,
            y_rot: 0.0,
            x_rot: 12.0,
        });
        assert_eq!(
            actor.inventory.lock().get_selected_item().count(),
            3,
            "the vanilla item-use fallback must not launch a second ball"
        );
        Ok(())
    })
    .await?;
    let record = DataPath::parse("creatures:state/party/0/entity")?;
    common::until(player, "native capture impact", || {
        player.data().get(&record).is_some()
    })
    .await?;
    assert!(cow.is_removed());
    assert_eq!(
        number(player, "magic:state/mana/current")?,
        35.0,
        "creature writes must preserve the independent mana compound"
    );
    common::until(player, "updated roster scoreboard", || {
        running
            .sent_packets
            .lock()
            .iter()
            .any(|packet| packet.encoded_data.windows(3).any(|bytes| bytes == b"cow"))
    })
    .await?;
    assert_eq!(player.hud_conflicts(), Vec::<String>::new());
    Ok(())
}

pub(super) async fn attack_spells(
    running: &common::Running,
    world: &Arc<World>,
) -> Result<(), String> {
    let player = &running.player;
    let server = &running.server;
    // Exercise each remaining damaging spell through its actual menu and impact.
    for index in [0, 2, 3, 6] {
        let world = Arc::clone(world);
        let target = ENTITIES
            .create(
                &vanilla_entities::COW,
                next_entity_id(),
                DVec3::new(8.5, 65.0, 9.5),
                Arc::downgrade(&world),
            )
            .ok_or("cow factory missing")?;
        let cow = Arc::clone(&target);
        let actor = Arc::clone(player);
        common::on_tick(server, move || {
            actor.data().set(
                &DataPath::parse("magic:state/mana/current")?,
                NbtTag::Double(100.0),
            )?;
            cow.as_mob().ok_or("cow should be a mob")?.set_no_ai(true);
            world.try_add_entity(cow).map_err(|e| e.to_string())
        })
        .await?;
        spell(server, player, index).await?;
        common::until(player, "damaging spell impact", || {
            target
                .as_living_entity()
                .is_some_and(|living| living.get_health() < 10.0)
        })
        .await?;
        assert!(number(player, "magic:state/mana/current")? < 100.0);
        if index != 2 {
            assert!(target.remaining_fire_ticks() > 0);
        }
        let cow = Arc::clone(&target);
        common::on_tick(server, move || {
            cow.set_removed(RemovalReason::Discarded);
            Ok(())
        })
        .await?;
    }
    Ok(())
}

pub(super) async fn terrain(running: &common::Running, world: &Arc<World>) -> Result<(), String> {
    let player = &running.player;
    let server = &running.server;
    let wall_world = Arc::clone(world);
    let actor = Arc::clone(player);
    common::on_tick(server, move || {
        actor.data().set(
            &DataPath::parse("magic:state/mana/current")?,
            NbtTag::Double(100.0),
        )?;
        // A real solid backdrop gives both terrain spells an exact impact face.
        for x in 7..=9 {
            for y in 65..=69 {
                wall_world.set_block(
                    BlockPos::new(x, y, 10),
                    vanilla_blocks::STONE.default_state(),
                    UpdateFlags::UPDATE_ALL,
                );
            }
        }
        // Remove fire left by the area spell before testing temporary terrain.
        wall_world.set_block(
            BlockPos::new(8, 65, 9),
            vanilla_blocks::AIR.default_state(),
            UpdateFlags::UPDATE_ALL,
        );
        Ok(())
    })
    .await?;
    let terrain = BlockPos::new(8, 65, 9);
    spell(server, player, 4).await?;
    common::until(player, "earth wall collision", || {
        world.get_block_state(terrain) == vanilla_blocks::DIRT.default_state()
    })
    .await?;
    common::until(player, "earth wall restoration", || {
        world.get_block_state(terrain) == vanilla_blocks::AIR.default_state()
    })
    .await?;
    spell(server, player, 5).await?;
    common::until(player, "water spring", || {
        world.get_block_state(terrain) == vanilla_blocks::WATER.default_state()
    })
    .await?;
    common::until(player, "water source restoration", || {
        world.get_block_state(terrain) != vanilla_blocks::WATER.default_state()
    })
    .await?;
    Ok(())
}

pub(super) async fn dependency_data(running: &common::Running) -> Result<(), String> {
    let player = &running.player;
    let server = &running.server;
    // The example reads the live gauge maximum and preserves dependency fields.
    let actor = Arc::clone(player);
    common::on_tick(server, move || {
        let mut data = actor.data();
        data.set(
            &DataPath::parse("magic:state/dependency")?,
            NbtTag::String("kept".into()),
        )?;
        data.set(
            &DataPath::parse("magic:state/mana/maximum")?,
            NbtTag::Double(80.0),
        )?;
        data.set(
            &DataPath::parse("magic:state/mana/current")?,
            NbtTag::Double(100.0),
        )
    })
    .await?;
    common::until(player, "dependency gauge update", || {
        number(player, "magic:state/mana/current") == Ok(80.0)
    })
    .await?;
    assert_eq!(
        player
            .data()
            .get(&DataPath::parse("magic:state/dependency")?),
        Some(&NbtTag::String("kept".into()))
    );
    Ok(())
}
