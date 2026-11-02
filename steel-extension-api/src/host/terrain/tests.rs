use super::*;
use crate::host::test_support::{fresh_test_world, insert_ready_full_chunk};
use steel_core::bootstrap::init_globals;
use steel_registry::vanilla_blocks;

#[test]
fn temporary_terrain_expires_and_preserves_a_later_edit_even_when_the_state_matches_again()
-> Result<(), String> {
    init_globals();
    let fixture = fresh_test_world("temporary_terrain_expiry");
    let world = &fixture.world;
    insert_ready_full_chunk(world, ChunkPos::new(0, 0));
    let first = BlockPos::new(2, 65, 2);
    let edited = BlockPos::new(3, 65, 2);
    let air = vanilla_blocks::AIR.default_state();
    let stone = vanilla_blocks::STONE.default_state();
    assert!(set(world, first, air, stone, 2)?);
    assert!(set(world, edited, air, stone, 2)?);
    assert!(
        set(
            world,
            first,
            stone,
            vanilla_blocks::OAK_PLANKS.default_state(),
            2
        )
        .is_err()
    );
    restore_expired(world, world.game_time() + 1)?;
    assert_eq!(world.get_block_state(first), stone);
    assert!(world.set_block(
        edited,
        vanilla_blocks::OAK_PLANKS.default_state(),
        UpdateFlags::UPDATE_ALL
    ));
    assert!(world.set_block(edited, stone, UpdateFlags::UPDATE_ALL));
    restore_expired(world, world.game_time() + 2)?;
    assert_eq!(world.get_block_state(first), air);
    assert_eq!(world.get_block_state(edited), stone);
    assert!(set(world, first, air, vanilla_blocks::CHEST.default_state(), 2).is_err());
    assert!(set(world, BlockPos::new(2000, 65, 2), air, stone, 2).is_err());
    Ok(())
}
