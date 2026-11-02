//! Owner-bound HUD contributions and player-data templates.
//! Independent sidebars and tab sections combine; boss bars have source-local identities.
//! The owner configures layout under `server.hud` and resolves overflow explicitly.

use crate::{PlayerId, Server, players::Uuid};
use stabby::{str::Str, string::String as ApiString, vec::Vec as ApiVec};
use std::sync::OnceLock;

/// A source registered under the calling extension's identity.
#[stabby::stabby]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub(crate) id: u64,
}

static DEFAULT_SOURCE: OnceLock<Source> = OnceLock::new();

#[doc(hidden)]
pub fn bind_default_source(hooks: &mut crate::Hooks<'_>) -> Result<(), ApiString> {
    let source = hooks.hud_source("default".into()).match_owned(Ok, Err)?;
    if *DEFAULT_SOURCE.get_or_init(|| source) == source {
        Ok(())
    } else {
        Err("default HUD source identity changed".into())
    }
}

fn default_source() -> Result<Source, ApiString> {
    DEFAULT_SOURCE
        .get()
        .copied()
        .ok_or_else(|| "HUD is unavailable before extension pre-init".into())
}

/// Vanilla boss bar color.
#[stabby::stabby]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarColor {
    /// Pink.
    Pink,
    /// Blue.
    Blue,
    /// Red.
    Red,
    /// Green.
    Green,
    /// Yellow.
    Yellow,
    /// Purple.
    Purple,
    /// White.
    White,
}

/// Vanilla boss bar segmentation.
#[stabby::stabby]
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarStyle {
    /// Continuous bar.
    Progress,
    /// Six segments.
    Notched6,
    /// Ten segments.
    Notched10,
    /// Twelve segments.
    Notched12,
    /// Twenty segments.
    Notched20,
}

/// A boss bar as shown to one player.
#[stabby::stabby]
#[derive(Debug, Clone, PartialEq)]
pub struct BossBar {
    /// Title (JSON text or plain).
    pub title: ApiString,
    /// Fill from 0 to 1.
    pub progress: f32,
    /// Color.
    pub color: BarColor,
    /// Segmentation.
    pub style: BarStyle,
    /// Darkens the sky.
    pub darken_screen: bool,
    /// Plays boss music.
    pub boss_music: bool,
    /// Adds fog.
    pub fog: bool,
}

impl BossBar {
    /// A plain bar with `title`, `progress` and `color`.
    #[must_use]
    pub fn new(title: &str, progress: f32, color: BarColor) -> Self {
        Self {
            title: title.into(),
            progress,
            color,
            style: BarStyle::Progress,
            darken_screen: false,
            boss_music: false,
            fog: false,
        }
    }

    /// Sets the segmentation.
    #[must_use]
    pub const fn style(mut self, style: BarStyle) -> Self {
        self.style = style;
        self
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "Borrowed ABI handles follow the shared receiver convention of the Server API"
)]
#[stabby::host_api]
/// HUD bindings on `Server`.
pub mod api {
    use super::{BossBar, Source};
    use crate::{ApiResult, ExtensionContext, Hooks, PlayerId, Server, players::Uuid};
    use stabby::{str::Str, string::String as ApiString, vec::Vec as ApiVec};

    #[cfg(feature = "host")]
    use crate::host::hud as host;

    impl<'a> Hooks<'a> {
        /// Registers a named source during pre-init.
        pub fn hud_source(&mut self, key: Str<'_>) -> ApiResult<Source> {
            host::source(self.host().extension.name, key.as_str())
                .map_err(ApiString::from)
                .into()
        }
    }

    impl<'a> ExtensionContext<'a> {
        /// Registers a named source under this extension's identity.
        pub fn hud_source(&self, key: Str<'_>) -> ApiResult<Source> {
            host::source(self.host().state.name, key.as_str())
                .map_err(ApiString::from)
                .into()
        }
    }

    impl Source {
        /// Registers a friendly alias for a typed player-data path.
        pub fn bind(&self, alias: Str<'_>, path: Str<'_>) -> ApiResult<()> {
            host::bind(*self, alias.as_str(), path.as_str())
                .map_err(ApiString::from)
                .into()
        }

        /// Publishes a sidebar section. Templates support `${alias}` and `${bar(alias, 20)}`.
        /// An empty title contributes lines without adding a header label or separator.
        pub fn set_sidebar(
            &self,
            server: &Server<'_>,
            player: PlayerId,
            title: Str<'_>,
            lines: ApiVec<ApiString>,
        ) -> ApiResult<()> {
            host::set_sidebar(server.host(), *self, player, title.as_str(), &lines)
                .map_err(ApiString::from)
                .into()
        }

        /// Removes only this source's sidebar section.
        pub fn clear_sidebar(&self, server: &Server<'_>, player: PlayerId) -> ApiResult<()> {
            host::clear_sidebar(server.host(), *self, player)
                .map_err(ApiString::from)
                .into()
        }

        /// Shows or updates a boss bar; only changed fields are sent.
        pub fn set_boss_bar(
            &self,
            server: &Server<'_>,
            player: PlayerId,
            id: Uuid,
            bar: BossBar,
        ) -> ApiResult<()> {
            host::set_boss_bar(server.host(), *self, player, id, &bar, None)
                .map_err(ApiString::from)
                .into()
        }

        /// Publishes a boss bar whose progress follows a gauge compound.
        pub fn set_gauge_bar(
            &self,
            server: &Server<'_>,
            player: PlayerId,
            id: Uuid,
            bar: BossBar,
            gauge: Str<'_>,
        ) -> ApiResult<()> {
            host::set_boss_bar(server.host(), *self, player, id, &bar, Some(gauge.as_str()))
                .map_err(ApiString::from)
                .into()
        }

        /// Removes only this source's bar with this local UUID.
        pub fn remove_boss_bar(
            &self,
            server: &Server<'_>,
            player: PlayerId,
            id: Uuid,
        ) -> ApiResult<()> {
            host::remove_boss_bar(server.host(), *self, player, id)
                .map_err(ApiString::from)
                .into()
        }

        /// Publishes a tab header/footer section alongside other sources and the server default.
        pub fn set_tab_list(
            &self,
            server: &Server<'_>,
            player: PlayerId,
            header: Str<'_>,
            footer: Str<'_>,
        ) -> ApiResult<()> {
            host::set_tab_list(
                server.host(),
                *self,
                player,
                header.as_str(),
                footer.as_str(),
            )
            .map_err(ApiString::from)
            .into()
        }

        /// Removes only this source's tab section.
        pub fn clear_tab_list(&self, server: &Server<'_>, player: PlayerId) -> ApiResult<()> {
            host::clear_tab_list(server.host(), *self, player)
                .map_err(ApiString::from)
                .into()
        }

        /// Removes all this source's contributions for one player.
        pub fn clear(&self, server: &Server<'_>, player: PlayerId) -> ApiResult<()> {
            host::clear(server.host(), *self, player)
                .map_err(ApiString::from)
                .into()
        }
    }

    impl<'a> Server<'a> {
        /// Current rendering conflicts, suitable for owner diagnostics.
        pub fn hud_conflicts(&self, player: PlayerId) -> ApiResult<ApiVec<ApiString>> {
            host::conflicts(self.host(), player)
                .map_err(ApiString::from)
                .into()
        }
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "Default HUD helpers follow the shared receiver convention of the Server API"
)]
impl Server<'_> {
    /// Publishes the extension's default sidebar section.
    #[must_use]
    pub fn set_sidebar(
        &self,
        player: PlayerId,
        title: Str<'_>,
        lines: ApiVec<ApiString>,
    ) -> crate::ApiResult<()> {
        match default_source() {
            Ok(source) => source.set_sidebar(self, player, title, lines),
            Err(error) => Err(error).into(),
        }
    }
    /// Removes this extension's default sidebar section.
    #[must_use]
    pub fn clear_sidebar(&self, player: PlayerId) -> crate::ApiResult<()> {
        match default_source() {
            Ok(source) => source.clear_sidebar(self, player),
            Err(error) => Err(error).into(),
        }
    }
    /// Publishes a source-local boss bar.
    #[must_use]
    pub fn set_boss_bar(&self, player: PlayerId, id: Uuid, bar: BossBar) -> crate::ApiResult<()> {
        match default_source() {
            Ok(source) => source.set_boss_bar(self, player, id, bar),
            Err(error) => Err(error).into(),
        }
    }
    /// Removes this extension's source-local boss bar.
    #[must_use]
    pub fn remove_boss_bar(&self, player: PlayerId, id: Uuid) -> crate::ApiResult<()> {
        match default_source() {
            Ok(source) => source.remove_boss_bar(self, player, id),
            Err(error) => Err(error).into(),
        }
    }
    /// Publishes this extension's default tab section.
    #[must_use]
    pub fn set_tab_list(
        &self,
        player: PlayerId,
        header: Str<'_>,
        footer: Str<'_>,
    ) -> crate::ApiResult<()> {
        match default_source() {
            Ok(source) => source.set_tab_list(self, player, header, footer),
            Err(error) => Err(error).into(),
        }
    }
    /// Removes this extension's default tab section.
    #[must_use]
    pub fn clear_tab_list(&self, player: PlayerId) -> crate::ApiResult<()> {
        match default_source() {
            Ok(source) => source.clear_tab_list(self, player),
            Err(error) => Err(error).into(),
        }
    }
}
