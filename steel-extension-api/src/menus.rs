//! Chest GUIs.
//!
//! [`Server::open_menu`](crate::Server::open_menu) shows a read-only chest of 1–6 rows filled
//! with [`ItemSpec`](crate::items::ItemSpec) icons. Items never move: every click is
//! cancelled and delivered to the menu's [`MenuHandler`] as a [`MenuClick`]. Slots
//! `0..rows*9` are the chest; higher slots are the player's inventory below it. Handlers may
//! give items, charge money, update icons with
//! [`Server::set_menu_item`](crate::Server::set_menu_item), open another menu or close it.
//!
//! Register a handler once (for example during init) and reuse it for every menu it serves:
//!
//! ```ignore
//! let handler = menus::handler(|server, click| { /* ... */ });
//! server.open_menu_with(player, "Enchants", 3, icons, handler)?;
//! ```

use crate::{PlayerId, Server, callbacks::Table, events::ClickKind, items::ItemSpec};

/// A per-view condition for interacting with a persistent block inventory.
#[stabby::stabby(checked)]
pub trait StorageAccess: Send + Sync {
    /// Checks the world without loading chunks or changing inventory state.
    extern "C" fn is_valid(&self, world: crate::World<'_>) -> bool;
}

/// Owned access check retained while the storage menu is open.
pub type DynStorageAccess = stabby::dynptr!(stabby::boxed::Box<dyn StorageAccess + Send + Sync>);

/// One click in a extension menu.
#[stabby::stabby]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuClick {
    /// Menu ID returned when it was opened.
    pub menu: u64,
    /// Clicking player.
    pub player: PlayerId,
    /// Clicked slot; -999 outside the window.
    pub slot: i32,
    /// Mouse button or hotbar key.
    pub button: i32,
    /// Click kind.
    pub kind: ClickKind,
}

/// One slot replacement in an open extension menu.
#[stabby::stabby]
pub struct MenuItemUpdate {
    /// Replacement stack, or an empty item to clear the slot.
    pub item: ItemSpec,
    /// Zero-based slot in the extension menu.
    pub slot: i32,
}

#[stabby::host_api]
/// Menu bindings on `Server`.
pub mod api {
    use super::{DynStorageAccess, MenuClick, MenuItemUpdate};
    use crate::{
        ApiResult, BlockPos, PlayerId, Server, block_behavior::types::Player, items::ItemSpec,
    };
    #[cfg(feature = "host")]
    use stabby::string::String as ApiString;
    use stabby::{str::Str, vec::Vec as ApiVec};

    #[cfg(feature = "host")]
    use crate::host::menus as host;

    impl Player<'_> {
        /// Opens a writable chest backed by the storage block at `pos`.
        pub fn open_storage(
            &self,
            pos: BlockPos,
            title: Str<'_>,
            slots: u32,
            access: DynStorageAccess,
        ) -> ApiResult<()> {
            crate::host::storage::open(self.host(), pos, pos, title.as_str(), slots, access)
                .map_err(ApiString::from)
                .into()
        }

        /// Opens a writable chest backed by `core`, with reach measured from
        /// the interacted block. Pages share one persistent inventory.
        pub fn open_storage_from(
            &self,
            core: BlockPos,
            interacted: BlockPos,
            title: Str<'_>,
            slots: u32,
            access: DynStorageAccess,
        ) -> ApiResult<()> {
            crate::host::storage::open(self.host(), core, interacted, title.as_str(), slots, access)
                .map_err(ApiString::from)
                .into()
        }

        /// Sends a plain system message to the interacting player.
        pub fn send_message(&self, message: Str<'_>) {
            self.host()
                .send_message(&text_components::TextComponent::plain(
                    message.as_str().to_owned(),
                ));
        }
    }

    impl<'a> Server<'a> {
        /// Opens a read-only chest GUI; `callback(server, click, token)` receives every click.
        /// Returns the menu ID. Prefer [`Server::open_menu_with`].
        pub fn open_menu(
            &self,
            player: PlayerId,
            title: Str<'_>,
            rows: i32,
            items: ApiVec<ItemSpec>,
            callback: extern "C" fn(Server<'_>, MenuClick, u64),
            token: u64,
        ) -> ApiResult<u64> {
            host::open_menu(
                self.host(),
                player,
                title.as_str(),
                rows,
                &items,
                callback,
                token,
            )
            .map_err(ApiString::from)
            .into()
        }

        /// Replaces one icon in the player's open extension menu.
        pub fn set_menu_item(&self, player: PlayerId, slot: i32, item: ItemSpec) -> ApiResult<()> {
            host::set_menu_item(self.host(), player, slot, &item)
                .map_err(ApiString::from)
                .into()
        }

        /// Updates several slots in the specified menu. Returns false when the
        /// player has disconnected, closed that menu, or opened another one.
        pub fn update_menu(
            &self,
            player: PlayerId,
            menu: u64,
            updates: ApiVec<MenuItemUpdate>,
        ) -> ApiResult<bool> {
            host::update_menu(self.host(), player, menu, &updates)
                .map_err(ApiString::from)
                .into()
        }

        /// Closes the player's extension menu, if one is open.
        pub fn close_menu(&self, player: PlayerId) -> ApiResult<()> {
            host::close_menu(self.host(), player)
                .map_err(ApiString::from)
                .into()
        }
    }
}

type ClickFn = dyn Fn(&Server<'_>, MenuClick) + Send + Sync;

static HANDLERS: Table<ClickFn> = Table::new();

extern "C" fn dispatch(server: Server<'_>, click: MenuClick, token: u64) {
    if let Some(handler) = HANDLERS.get(token) {
        handler(&server, click);
    }
}

/// A registered click handler, reusable for any number of menus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuHandler {
    token: u64,
}

/// Registers a click handler for the extension's lifetime.
pub fn handler(f: impl Fn(&Server<'_>, MenuClick) + Send + Sync + 'static) -> MenuHandler {
    MenuHandler {
        token: HANDLERS.insert(Box::new(f)),
    }
}

impl Server<'_> {
    /// Opens a read-only chest GUI handled by `handler`; returns the menu ID.
    ///
    /// # Errors
    ///
    /// Returns the host diagnostic for invalid rows, items or an offline player.
    pub fn open_menu_with(
        &self,
        player: PlayerId,
        title: &str,
        rows: i32,
        items: impl IntoIterator<Item = ItemSpec>,
        handler: MenuHandler,
    ) -> Result<u64, String> {
        Result::from(self.open_menu(
            player,
            title.into(),
            rows,
            items.into_iter().collect(),
            dispatch,
            handler.token,
        ))
        .map_err(|error| error.to_string())
    }
}
