use std::io::{Result, Write};

use steel_macros::{ClientPacket, ReadFrom, ServerPacket};
use steel_registry::packets::{config, play};
use steel_utils::{
    codec::VarInt,
    serial::{PrefixedWrite, WriteTo},
};
use text_components::TextComponent;
use uuid::Uuid;

/// Default string limit used by vanilla's resource pack URL codec.
pub const MAX_RESOURCE_PACK_URL_LENGTH: usize = i16::MAX as usize;
const MAX_HASH_LENGTH: usize = 40;

/// Offers a server resource pack during configuration or play.
#[derive(ClientPacket, Clone, Debug)]
#[packet_id(Config = config::C_RESOURCE_PACK_PUSH, Play = play::C_RESOURCE_PACK_PUSH)]
pub struct CResourcePackPush {
    pub id: Uuid,
    pub url: String,
    pub hash: String,
    pub required: bool,
    pub prompt: Option<TextComponent>,
}

impl WriteTo for CResourcePackPush {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        self.id.write(writer)?;
        self.url
            .write_prefixed_bound::<VarInt>(writer, MAX_RESOURCE_PACK_URL_LENGTH)?;
        self.hash
            .write_prefixed_bound::<VarInt>(writer, MAX_HASH_LENGTH)?;
        self.required.write(writer)?;
        self.prompt.write(writer)
    }
}

/// A response to a particular resource pack. Enum order is the protocol order.
#[derive(ReadFrom, ServerPacket, Debug)]
pub struct SResourcePack {
    pub id: Uuid,
    pub action: ResourcePackAction,
}

#[derive(ReadFrom, Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourcePackAction {
    SuccessfullyLoaded = 0,
    Declined = 1,
    FailedDownload = 2,
    Accepted = 3,
    Downloaded = 4,
    InvalidUrl = 5,
    FailedReload = 6,
    Discarded = 7,
}

impl ResourcePackAction {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Accepted | Self::Downloaded)
    }
}
