//! Host side of the per-player HUD API.
use super::{events::parse_text, players::find};
use crate::{
    PlayerId,
    hud::{BarColor, BarStyle, BossBar, Source},
    players::Uuid,
};
use stabby::{string::String as ApiString, vec::Vec as ApiVec};
use steel_core::{player::hud, server::Server as CoreServer};
use steel_protocol::packets::game::{BossBarColor, BossBarOverlay};

const fn color(color: BarColor) -> BossBarColor {
    match color {
        BarColor::Pink => BossBarColor::Pink,
        BarColor::Blue => BossBarColor::Blue,
        BarColor::Red => BossBarColor::Red,
        BarColor::Green => BossBarColor::Green,
        BarColor::Yellow => BossBarColor::Yellow,
        BarColor::Purple => BossBarColor::Purple,
        BarColor::White => BossBarColor::White,
    }
}

const fn overlay(style: BarStyle) -> BossBarOverlay {
    match style {
        BarStyle::Progress => BossBarOverlay::Progress,
        BarStyle::Notched6 => BossBarOverlay::Notched6,
        BarStyle::Notched10 => BossBarOverlay::Notched10,
        BarStyle::Notched12 => BossBarOverlay::Notched12,
        BarStyle::Notched20 => BossBarOverlay::Notched20,
    }
}

const fn uuid(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.bytes)
}

pub(crate) fn set_sidebar(
    server: &CoreServer,
    owner: Source,
    id: PlayerId,
    title: &str,
    lines: &[ApiString],
) -> Result<(), String> {
    find(server, id)?.set_hud_sidebar(
        &hud::catalog::source(owner.id)?,
        Some(hud::SidebarSection {
            title: title.to_owned(),
            lines: lines.iter().map(ToString::to_string).collect(),
        }),
    )
}

pub(crate) fn clear_sidebar(
    server: &CoreServer,
    owner: Source,
    id: PlayerId,
) -> Result<(), String> {
    find(server, id)?.set_hud_sidebar(&hud::catalog::source(owner.id)?, None)
}

pub(crate) fn set_boss_bar(
    server: &CoreServer,
    owner: Source,
    id: PlayerId,
    bar_id: Uuid,
    bar: &BossBar,
    gauge: Option<&str>,
) -> Result<(), String> {
    let flags =
        u8::from(bar.darken_screen) | u8::from(bar.boss_music) << 1 | u8::from(bar.fog) << 2;
    find(server, id)?.set_hud_bar(
        &hud::catalog::source(owner.id)?,
        uuid(bar_id),
        Some(hud::BarContribution {
            title: bar.title.to_string(),
            gauge: gauge.map(str::to_owned),
            bar: hud::BossBar {
                title: parse_text(bar.title.as_str()),
                progress: bar.progress,
                color: color(bar.color),
                overlay: overlay(bar.style),
                flags,
            },
        }),
    )
}

pub(crate) fn remove_boss_bar(
    server: &CoreServer,
    owner: Source,
    id: PlayerId,
    bar_id: Uuid,
) -> Result<(), String> {
    find(server, id)?.set_hud_bar(&hud::catalog::source(owner.id)?, uuid(bar_id), None)
}

pub(crate) fn set_tab_list(
    server: &CoreServer,
    owner: Source,
    id: PlayerId,
    header: &str,
    footer: &str,
) -> Result<(), String> {
    find(server, id)?.set_hud_tab(
        &hud::catalog::source(owner.id)?,
        Some(hud::TabSection {
            header: header.to_owned(),
            footer: footer.to_owned(),
        }),
    )
}

pub(crate) fn clear_tab_list(
    server: &CoreServer,
    owner: Source,
    id: PlayerId,
) -> Result<(), String> {
    find(server, id)?.set_hud_tab(&hud::catalog::source(owner.id)?, None)
}

pub(crate) fn source(owner: &str, key: &str) -> Result<Source, String> {
    hud::catalog::register(owner, key).map(|id| Source { id })
}

pub(crate) fn bind(source: Source, alias: &str, path: &str) -> Result<(), String> {
    hud::catalog::bind(source.id, alias, path)
}

pub(crate) fn clear(server: &CoreServer, source: Source, player: PlayerId) -> Result<(), String> {
    find(server, player)?.clear_hud_source(&hud::catalog::source(source.id)?)
}

pub(crate) fn conflicts(
    server: &CoreServer,
    player: PlayerId,
) -> Result<ApiVec<ApiString>, String> {
    Ok(find(server, player)?
        .hud_conflicts()
        .into_iter()
        .map(ApiString::from)
        .collect())
}
