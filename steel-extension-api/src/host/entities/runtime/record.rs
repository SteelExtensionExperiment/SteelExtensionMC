use simdnbt::owned::{NbtCompound, NbtTag};
use steel_core::entity::Entity;
use steel_core::{
    chunk_saver::{ChunkStorage, PersistentEntity},
    entity::SharedEntity,
    entity::custom_data::DataPath,
    player::Player,
};

pub(super) fn snapshot(entity: &SharedEntity) -> Result<NbtCompound, String> {
    let saved = ChunkStorage::owned_entity_to_persistent(entity)
        .ok_or("entity has no valid native snapshot")?;
    let bytes = wincode::serialize(&saved).map_err(|e| e.to_string())?;
    let mut record = NbtCompound::new();
    record.insert("entity", NbtTag::ByteArray(bytes));
    record.insert(
        "kind",
        NbtTag::String(entity.entity_type().key.to_string().into()),
    );
    record.insert("uuid", NbtTag::String(entity.uuid().to_string().into()));
    if let Some(living) = entity.as_living_entity() {
        let mut gauge = NbtCompound::new();
        gauge.insert("current", NbtTag::Double(f64::from(living.get_health())));
        gauge.insert(
            "maximum",
            NbtTag::Double(f64::from(living.get_max_health())),
        );
        record.insert("health", NbtTag::Compound(gauge));
    }
    record.insert("deployed", NbtTag::Byte(1));
    Ok(record)
}

pub(super) fn read(player: &Player, path: &str) -> Result<PersistentEntity, String> {
    let path = DataPath::parse(&format!("{path}/entity"))?;
    let data = player.data();
    let bytes = data
        .get(&path)
        .and_then(NbtTag::byte_array)
        .ok_or("entity slot has no native snapshot")?;
    wincode::deserialize(bytes).map_err(|e| e.to_string())
}
