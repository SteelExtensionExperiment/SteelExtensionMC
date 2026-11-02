use super::*;
use crate::chunk::block_attachment::BlockAttachment;
#[test]
fn block_attachments_roundtrip_with_named_states_and_reject_corrupt_references()
-> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("block_attachment_persistence");
    let world = &fixture.world;
    let pos = ChunkPos::new(0, 0);
    let holder = insert_ready_full_chunk(world, pos);
    let block = BlockPos::new(2, 65, 2);
    let air = vanilla_blocks::AIR.default_state();
    let stone = vanilla_blocks::STONE.default_state();
    let key = Identifier::new_static("test", "claim");
    let payload = b"owner-defined data".to_vec();
    assert!(matches!(
        world.set_block_with_attachment(
            block,
            air,
            stone,
            UpdateFlags::UPDATE_ALL,
            Arc::new(BlockAttachment {
                key: key.clone(),
                states: vec![air, stone],
                data: payload.clone()
            })
        ),
        crate::world::ConditionalBlockSetResult::Changed
    ));
    let chunk = holder.try_chunk(ChunkStatus::Full).ok_or("chunk missing")?;
    let saved = ChunkStorage::prepare_chunk_save(chunk, ChunkStatus::Full, &[], true)
        .ok_or("snapshot missing")?;
    let bytes = wincode::serialize(&saved.persistent).map_err(|e| e.to_string())?;
    let mut decoded: PersistentChunk<'_> =
        wincode::deserialize(&bytes).map_err(|e| e.to_string())?;
    let loaded = ChunkStorage::try_persistent_to_chunk(
        &decoded,
        pos,
        ChunkStatus::Full,
        chunk.min_y(),
        chunk.height(),
        Arc::downgrade(world),
    )
    .map_err(|e| e.to_string())?;
    assert_eq!(loaded.chunk.get_block_state(block), stone);
    let records = loaded.chunk.block_attachments.lock();
    let record = records.get(&block).ok_or("restoration missing")?;
    assert_eq!(record.states, [air, stone]);
    assert_eq!(record.key, key);
    assert_eq!(record.data, payload);
    drop(records);
    decoded.block_attachments[0].states[0] = u16::MAX;
    assert!(
        ChunkStorage::try_persistent_to_chunk(
            &decoded,
            pos,
            ChunkStatus::Full,
            chunk.min_y(),
            chunk.height(),
            Arc::downgrade(world)
        )
        .is_err()
    );
    Ok(())
}
