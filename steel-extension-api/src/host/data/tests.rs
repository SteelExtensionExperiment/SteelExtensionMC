use super::*;
use crate::{
    ApiResult, Server as ApiServer, World as ApiWorld,
    block_behavior::types::Entity as EntityView,
    data::{Store, Value},
    host::test_support::{common, fresh_test_world},
    players::Uuid,
};
use glam::DVec3;
use simdnbt::owned::NbtCompound;
use stabby::{abi::IDeterminantProvider, string::String as ApiString};
use steel_core::{
    bootstrap::init_globals,
    chunk_saver::{ChunkStorage, PersistentEntity},
    entity::{ENTITIES, Entity, LivingEntity, RemovalReason, next_entity_id},
    player::player_data::PersistentPlayerData,
};
use steel_registry::vanilla_entities;

fn api<T: IDeterminantProvider<ApiString>>(result: ApiResult<T>) -> Result<T, String> {
    result
        .match_owned(Ok, Err)
        .map_err(|error| error.to_string())
}

fn identity(entity: &dyn Entity) -> EntityId {
    EntityId {
        id: entity.id(),
        uuid: Uuid {
            bytes: *entity.uuid().as_bytes(),
        },
    }
}

fn number(store: &Store, path: &str) -> Result<f64, String> {
    let value: Option<Value> = api(store.get(path.into()))?.into();
    value
        .and_then(|value| Option::<f64>::from(value.as_number()))
        .ok_or("numeric field missing".into())
}

#[test]
fn live_entity_data_merges_preserves_native_fields_and_rejects_stale_targets() -> Result<(), String>
{
    init_globals();
    let fixture = fresh_test_world("shared_entity_data");
    let world = &fixture.world;
    let entity = ENTITIES
        .create(
            &vanilla_entities::COW,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        )
        .ok_or("cow factory missing")?;
    let mut native = NbtCompound::new();
    native.insert("native_flag", NbtTag::Byte(1));
    entity.set_custom_data(native);
    world
        .try_add_entity(Arc::clone(&entity))
        .map_err(|e| e.to_string())?;
    let data = api(ApiWorld::from_host(world).entity_data(identity(entity.as_ref())))?;
    let borrowed = EntityView::from_host(entity.as_ref());
    let other = api(borrowed.data())?;
    api(data.initialize("example:state/energy".into(), Value::integer(20)))?;
    api(other.set("dependency:state/value".into(), Value::integer(7)))?;
    assert!(api(other.spend("example:state/energy".into(), 5.0))?);
    assert!(!api(data.spend("example:state/energy".into(), 50.0))?);
    assert_eq!(number(&data, "example:state/energy")?, 15.0);
    assert_eq!(number(&data, "dependency:state/value")?, 7.0);
    assert_eq!(entity.custom_data().byte("native_flag"), Some(1));
    assert!(api(data.set("example:state/energy/child".into(), Value::integer(1))).is_err());
    assert_eq!(number(&other, "example:state/energy")?, 15.0);
    entity.set_removed(RemovalReason::Discarded);
    assert!(api(data.get("example:state/energy".into())).is_err());
    Ok(())
}

#[test]
fn custom_data_survives_native_entity_save_load_and_conversion() -> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("saved_entity_data");
    let world = &fixture.world;
    let entity = ENTITIES
        .create(
            &vanilla_entities::COW,
            next_entity_id(),
            DVec3::new(8.5, 65.0, 8.5),
            Arc::downgrade(world),
        )
        .ok_or("cow factory missing")?;
    world
        .try_add_entity(Arc::clone(&entity))
        .map_err(|e| e.to_string())?;
    let data = api(ApiWorld::from_host(world).entity_data(identity(entity.as_ref())))?;
    api(data.set("example:state/energy".into(), Value::integer(17)))?;
    let saved =
        ChunkStorage::owned_entity_to_persistent(&entity).ok_or("native snapshot missing")?;
    let bytes = wincode::serialize(&saved).map_err(|e| e.to_string())?;
    let saved: PersistentEntity = wincode::deserialize(&bytes).map_err(|e| e.to_string())?;
    entity.set_removed(RemovalReason::Discarded);
    let restored = ChunkStorage::persistent_to_entity_at_level(
        &saved,
        ChunkPos::new(0, 0),
        &Arc::downgrade(world),
    )
    .ok_or("native entity could not be restored")?;
    world
        .try_add_entity(Arc::clone(&restored))
        .map_err(|e| e.to_string())?;
    let restored_data = api(ApiWorld::from_host(world).entity_data(identity(restored.as_ref())))?;
    assert_eq!(number(&restored_data, "example:state/energy")?, 17.0);
    let converted = steel_core::entity::convert_mob(
        restored.as_mob().ok_or("restored mob missing")?,
        &vanilla_entities::PIG,
        steel_core::entity::ConversionParams::single(false, false),
        None,
    )
    .ok_or("conversion failed")?;
    let converted_data = api(ApiWorld::from_host(world).entity_data(identity(converted.as_ref())))?;
    assert_eq!(number(&converted_data, "example:state/energy")?, 17.0);
    assert!(api(restored_data.get("example:state/energy".into())).is_err());
    Ok(())
}

#[test]
fn player_and_entity_access_share_data_preserved_by_respawn_and_domain_snapshots()
-> Result<(), String> {
    let root = tempfile::tempdir().map_err(|e| e.to_string())?;
    let loaded = common::load_extensions(&[], root.path())?;
    let fixture = common::Fixture::new()?;
    let runtime = Arc::clone(&fixture.runtime);
    runtime.block_on(async {
        let uuid = uuid::Uuid::new_v4();
        let running = common::start(
            &fixture.world,
            (loaded.extensions, loaded.commands),
            common::grants(uuid, &[])?,
            uuid,
            "DataOwner",
        )
        .await?;
        let server = Arc::clone(&running.server);
        let actor = Arc::clone(&running.player);
        let world = Arc::clone(&fixture.world);
        let targets = common::on_tick(&running.server, move || {
            let id = crate::host::events::player_id(&actor);
            let player_data = api(ApiServer::from_host(&server).player_data(id))?;
            let entity_data =
                api(ApiWorld::from_host(&world).entity_data(identity(actor.as_ref())))?;
            api(player_data.set("example:state/energy".into(), Value::integer(20)))?;
            api(entity_data.spend("example:state/energy".into(), 5.0))?;
            assert_eq!(number(&player_data, "example:state/energy")?, 15.0);
            let snapshot = PersistentPlayerData::from_player(&actor)?;
            let replacement = actor.new_respawn_replacement(Arc::clone(&world), false, false, true);
            assert_eq!(
                replacement
                    .data()
                    .get(&DataPath::parse("example:state/energy")?)
                    .and_then(CustomData::number),
                Some(15.0)
            );
            replacement
                .data()
                .set(&DataPath::parse("example:state/energy")?, NbtTag::Int(99))?;
            assert_eq!(number(&player_data, "example:state/energy")?, 15.0);
            snapshot.apply_to_player_without_location(&replacement);
            assert_eq!(replacement.custom_data(), actor.custom_data());
            actor.set_health(0.0);
            actor.respawn();
            Ok((player_data.into_host(), entity_data.into_host()))
        })
        .await;
        let result = async {
            let (player_target, entity_target) = targets?;
            let generation = running.player.generation();
            common::until(&running.player, "data owner respawn", || {
                running.server.get_players().iter().any(|player| {
                    player.gameprofile.id() == uuid
                        && player.generation() != generation
                        && !player.is_world_change_pending()
                })
            })
            .await?;
            let server = Arc::clone(&running.server);
            common::on_tick(&running.server, move || {
                let replacement = server
                    .get_players()
                    .into_iter()
                    .find(|player| player.gameprofile.id() == uuid)
                    .ok_or("respawn replacement missing")?;
                let player_data = Store::from_host(player_target);
                let entity_data = Store::from_host(entity_target);
                assert!(api(player_data.get("example:state/energy".into())).is_err());
                assert!(
                    api(entity_data.set("example:state/energy".into(), Value::integer(42)))
                        .is_err()
                );
                let current = api(ApiServer::from_host(&server)
                    .player_data(crate::host::events::player_id(&replacement)))?;
                assert_eq!(number(&current, "example:state/energy")?, 15.0);
                Ok(())
            })
            .await
        }
        .await;
        common::stop(running, &loaded.initialized, &fixture.world).await?;
        result
    })
}
