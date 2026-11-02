//! Persistent, owner-defined data tied to one exact block-state claim.

use steel_utils::{BlockStateId, Identifier};

/// Installed atomically with a block write and removed by the next state change.
/// State references use the chunk's named palette on disk. Payload bytes are opaque
/// to the world and are interpreted only by the owner identified by `key`.
#[derive(Debug)]
pub struct BlockAttachment {
    /// Namespaced identity of the attachment's data format.
    pub key: Identifier,
    /// Referenced states, remapped through the chunk palette during save/load.
    pub states: Vec<BlockStateId>,
    /// Owner-defined persistent payload.
    pub data: Vec<u8>,
}
