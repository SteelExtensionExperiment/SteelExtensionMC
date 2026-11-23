//! Lossless player inventory capture and restore.
//!
//! [`Server::inventory_snapshot`](crate::Server::inventory_snapshot) captures inventory slots
//! 0-40 (hotbar, main, armor, off hand, as in [`items`](crate::items)), the selected hotbar slot
//! and the game mode. [`Server::restore_inventory`](crate::Server::restore_inventory) puts them
//! back and resyncs the client.
//!
//! Stacks are stored with Steel's persistent item codec, so every persistent data component
//! survives. [`ItemSpec`](crate::items::ItemSpec) stays lossy by design and is not used here.
//! Components that vanilla does not persist (such as `creative_slot_lock`) are dropped, as when
//! saving a player.
//!
//! Failures are strict: a stack that cannot be encoded fails the capture and names its slot; a
//! snapshot with an unknown format, unknown item, invalid components, an out-of-range or duplicate
//! slot, a bad selected slot or an unknown game mode is rejected as a whole and the player is left
//! unchanged. Inventory slots 41-42, experience and the ender chest are not part of a snapshot.
//!
//! The bytes in [`InventorySnapshot::data`] are opaque; persist them as they are. The restored
//! game mode goes through `set_game_mode`, so a cancelled game mode change event keeps the old mode.

use stabby::vec::Vec as ApiVec;

/// Snapshot format understood by this host.
pub const INVENTORY_SNAPSHOT_FORMAT: u32 = 1;

/// A captured inventory. Produced by [`Server::inventory_snapshot`](crate::Server::inventory_snapshot).
#[stabby::stabby]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventorySnapshot {
    /// [`INVENTORY_SNAPSHOT_FORMAT`] when produced by this host.
    pub format: u32,
    /// Binary NBT. Opaque to extensions.
    pub data: ApiVec<u8>,
}

#[stabby::host_api]
/// Inventory snapshot bindings on `Server`.
pub mod api {
    use super::InventorySnapshot;
    use crate::{ApiResult, PlayerId, Server};
    #[cfg(feature = "host")]
    use stabby::string::String as ApiString;

    #[cfg(feature = "host")]
    use crate::host::inventory_snapshot as host;

    impl<'a> Server<'a> {
        /// Captures slots 0-40, the selected hotbar slot and the game mode of a player.
        pub fn inventory_snapshot(&self, player: PlayerId) -> ApiResult<InventorySnapshot> {
            host::inventory_snapshot(self.host(), player)
                .map_err(ApiString::from)
                .into()
        }

        /// Replaces slots 0-40, the selected hotbar slot and the game mode from a snapshot.
        /// Slots absent from the snapshot are emptied. On error the player is unchanged.
        pub fn restore_inventory(
            &self,
            player: PlayerId,
            snapshot: &InventorySnapshot,
        ) -> ApiResult<()> {
            host::restore_inventory(self.host(), player, snapshot)
                .map_err(ApiString::from)
                .into()
        }
    }
}
