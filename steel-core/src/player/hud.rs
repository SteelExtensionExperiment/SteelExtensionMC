//! Per-connection HUD composition using ordinary vanilla packets.
//! Sources publish independent sections and local boss-bar identities; owner policy
//! controls the combined display. State survives respawns on `PlayerSession`.

pub mod catalog;
mod composition;
mod expression;
mod ownership;

pub use composition::{BarContribution, BossBarPolicy, HudPolicy, SidebarSection, TabSection};

use rustc_hash::FxHashMap;
use steel_protocol::packets::game::{
    BossBarColor, BossBarOverlay, BossEventOperation, CBossEvent, CResetScore,
    CSetDisplayObjective, CSetObjective, CSetScore, CTabList, DisplaySlot, NumberFormat,
    ObjectiveMethod,
};
use text_components::TextComponent;
use uuid::Uuid;

use crate::player::Player;

/// Name of the private sidebar objective.
const SIDEBAR_OBJECTIVE: &str = "steel_sidebar";
/// Lines the vanilla client renders in a sidebar.
pub const MAX_SIDEBAR_LINES: usize = 15;

/// A boss bar as shown to one player.
#[derive(Debug, Clone, PartialEq)]
pub struct BossBar {
    /// Title.
    pub title: TextComponent,
    /// Fill, clamped to 0..=1.
    pub progress: f32,
    /// Color.
    pub color: BossBarColor,
    /// Segment style.
    pub overlay: BossBarOverlay,
    /// Vanilla flag bits: 1 darken screen, 2 boss music, 4 fog.
    pub flags: u8,
}

/// One HUD packet; kept as data so the diffing is testable without a connection.
#[derive(Debug, Clone)]
pub(crate) enum HudPacket {
    Objective(CSetObjective),
    Display(CSetDisplayObjective),
    Score(CSetScore),
    Reset(CResetScore),
    Boss(CBossEvent),
    Tab(Box<(TextComponent, TextComponent)>),
}

#[derive(Debug, Clone)]
struct Sidebar {
    title: TextComponent,
    lines: Vec<TextComponent>,
}

/// HUD state of one client connection.
#[derive(Debug, Default)]
pub struct PlayerHud {
    rendered_revision: Option<(u64, u64)>,
    composition: composition::Composition,
    sidebar: Option<Sidebar>,
    boss_bars: FxHashMap<Uuid, BossBar>,
    tab_list: Option<(TextComponent, TextComponent)>,
}

fn line_owner(index: usize) -> String {
    // Vanilla hides score holders starting with '#', even with display text.
    format!("steel:sidebar/{index}")
}

impl PlayerHud {
    /// Applies a composed sidebar, already validated against the client limit.
    pub(crate) fn set_sidebar(
        &mut self,
        title: TextComponent,
        lines: &[TextComponent],
    ) -> Vec<HudPacket> {
        debug_assert!(lines.len() <= MAX_SIDEBAR_LINES);
        let mut packets = Vec::new();
        let previous = match self.sidebar.take() {
            None => {
                packets.push(HudPacket::Objective(CSetObjective {
                    name: SIDEBAR_OBJECTIVE.into(),
                    method: ObjectiveMethod::Add {
                        display_name: title.clone(),
                        number_format: Some(NumberFormat::Blank),
                    },
                }));
                packets.push(HudPacket::Display(CSetDisplayObjective {
                    slot: DisplaySlot::Sidebar,
                    objective: SIDEBAR_OBJECTIVE.into(),
                }));
                Vec::new()
            }
            Some(previous) => {
                if previous.title != title {
                    packets.push(HudPacket::Objective(CSetObjective {
                        name: SIDEBAR_OBJECTIVE.into(),
                        method: ObjectiveMethod::Change {
                            display_name: title.clone(),
                            number_format: Some(NumberFormat::Blank),
                        },
                    }));
                }
                previous.lines
            }
        };
        for (index, line) in lines.iter().enumerate() {
            if previous.get(index) == Some(line) {
                continue;
            }
            let Ok(score) = i32::try_from(MAX_SIDEBAR_LINES - index) else {
                continue;
            };
            packets.push(HudPacket::Score(CSetScore {
                owner: line_owner(index),
                objective: SIDEBAR_OBJECTIVE.into(),
                score,
                display: Some(line.clone()),
                number_format: None,
            }));
        }
        for index in lines.len()..previous.len() {
            packets.push(HudPacket::Reset(CResetScore {
                owner: line_owner(index),
                objective: Some(SIDEBAR_OBJECTIVE.into()),
            }));
        }
        self.sidebar = Some(Sidebar {
            title,
            lines: lines.to_vec(),
        });
        packets
    }

    /// Removes the sidebar.
    pub(crate) fn clear_sidebar(&mut self) -> Vec<HudPacket> {
        if self.sidebar.take().is_none() {
            return Vec::new();
        }
        vec![HudPacket::Objective(CSetObjective {
            name: SIDEBAR_OBJECTIVE.into(),
            method: ObjectiveMethod::Remove,
        })]
    }

    /// Shows or updates a boss bar.
    pub(crate) fn set_boss_bar(&mut self, id: Uuid, mut bar: BossBar) -> Vec<HudPacket> {
        bar.progress = bar.progress.clamp(0.0, 1.0);
        let boss = |operation| HudPacket::Boss(CBossEvent { id, operation });
        let Some(previous) = self.boss_bars.insert(id, bar.clone()) else {
            return vec![boss(BossEventOperation::Add {
                title: bar.title,
                progress: bar.progress,
                color: bar.color,
                overlay: bar.overlay,
                flags: bar.flags,
            })];
        };
        let mut packets = Vec::new();
        if previous.title != bar.title {
            packets.push(boss(BossEventOperation::UpdateName(bar.title)));
        }
        if previous.progress.to_bits() != bar.progress.to_bits() {
            packets.push(boss(BossEventOperation::UpdateProgress(bar.progress)));
        }
        if previous.color != bar.color || previous.overlay != bar.overlay {
            packets.push(boss(BossEventOperation::UpdateStyle(
                bar.color,
                bar.overlay,
            )));
        }
        if previous.flags != bar.flags {
            packets.push(boss(BossEventOperation::UpdateProperties(bar.flags)));
        }
        packets
    }

    /// Removes a boss bar.
    pub(crate) fn remove_boss_bar(&mut self, id: Uuid) -> Vec<HudPacket> {
        if self.boss_bars.remove(&id).is_none() {
            return Vec::new();
        }
        vec![HudPacket::Boss(CBossEvent {
            id,
            operation: BossEventOperation::Remove,
        })]
    }

    /// Whether an extension contributes tab content.
    pub(crate) fn has_tab_list_override(&self) -> bool {
        self.composition
            .sources
            .values()
            .any(|source| source.tab.is_some())
    }
}

impl Player {
    fn send_hud(&self, packets: Vec<HudPacket>) {
        for packet in packets {
            match packet {
                HudPacket::Objective(packet) => self.send_packet(packet),
                HudPacket::Display(packet) => self.send_packet(packet),
                HudPacket::Score(packet) => self.send_packet(packet),
                HudPacket::Reset(packet) => self.send_packet(packet),
                HudPacket::Boss(packet) => self.send_packet(packet),
                HudPacket::Tab(pair) => {
                    self.send_packet(CTabList::new(&pair.0, &pair.1, self));
                }
            }
        }
    }

    /// Whether extensions have published HUD contributions on this connection.
    #[must_use]
    pub fn has_tab_list_override(&self) -> bool {
        self.session.hud.lock().has_tab_list_override()
    }
}

#[cfg(test)]
#[path = "hud/tests.rs"]
mod composition_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> TextComponent {
        TextComponent::plain(value.to_owned())
    }

    fn bar(progress: f32) -> BossBar {
        BossBar {
            title: text("Mine"),
            progress,
            color: BossBarColor::Yellow,
            overlay: BossBarOverlay::Progress,
            flags: 0,
        }
    }

    #[test]
    fn sidebar_sends_only_changed_lines_and_resets_removed_ones() {
        let mut hud = PlayerHud::default();
        let first = hud.set_sidebar(text("Prison"), &[text("a"), text("b"), text("c")]);
        // add objective, display slot, three scores
        assert_eq!(first.len(), 5);
        let second = hud.set_sidebar(text("Prison"), &[text("a"), text("B")]);
        assert!(matches!(
            second.as_slice(),
            [HudPacket::Score(score), HudPacket::Reset(reset)]
                if score.owner == "steel:sidebar/1" && reset.owner == "steel:sidebar/2"
        ));
        assert!(
            hud.set_sidebar(text("Prison"), &[text("a"), text("B")])
                .is_empty()
        );
        assert_eq!(hud.clear_sidebar().len(), 1);
        assert!(hud.clear_sidebar().is_empty());
    }

    #[test]
    fn boss_bar_updates_send_one_operation_per_changed_field() {
        let mut hud = PlayerHud::default();
        let id = Uuid::from_u128(7);
        assert!(matches!(
            hud.set_boss_bar(id, bar(2.0)).as_slice(),
            [HudPacket::Boss(CBossEvent { operation: BossEventOperation::Add { progress, .. }, .. })]
                if (*progress - 1.0).abs() < f32::EPSILON
        ));
        assert!(hud.set_boss_bar(id, bar(1.0)).is_empty());
        assert!(matches!(
            hud.set_boss_bar(id, bar(0.5)).as_slice(),
            [HudPacket::Boss(CBossEvent {
                operation: BossEventOperation::UpdateProgress(_),
                ..
            })]
        ));
        assert_eq!(hud.remove_boss_bar(id).len(), 1);
        assert!(hud.remove_boss_bar(id).is_empty());
    }
}
