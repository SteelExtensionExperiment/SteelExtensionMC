//! Real, temporary block changes. Native state updates and collision remain active.

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "World bindings consistently borrow their capability handle"
)]
#[stabby::host_api]
/// Temporary terrain operations on a loaded world.
pub mod api {
    use crate::{ApiResult, BlockPos, BlockState, World};
    #[cfg(feature = "host")]
    use stabby::string::String as ApiString;
    impl<'a> World<'a> {
        /// Claims a loaded position only if its state matches. Expiry survives
        /// unload/restart; an intervening state change cancels restoration.
        /// Block entities and overlapping temporary claims are rejected.
        pub fn set_temporary_block(
            &self,
            pos: BlockPos,
            expected: BlockState,
            placed: BlockState,
            duration: u32,
        ) -> ApiResult<bool> {
            crate::host::terrain::set(
                self.host(),
                steel_utils::BlockPos::new(pos.x, pos.y, pos.z),
                steel_utils::BlockStateId(expected.id),
                steel_utils::BlockStateId(placed.id),
                duration,
            )
            .map_err(ApiString::from)
            .into()
        }
    }
}
