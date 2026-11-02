//! Owner-bound mutation and reconciliation of the connection's HUD.

use crate::entity::Entity;
use std::sync::Arc;
use text_components::TextComponent;
use uuid::Uuid;

use super::{
    BarContribution, HudPacket, PlayerHud, SidebarSection, TabSection,
    catalog::HudSource,
    composition::{HudPolicy, OwnedBar},
};
use crate::{entity::custom_data::CustomData, player::Player};

impl PlayerHud {
    #[cfg(test)]
    pub(super) fn reconcile_for_test(
        &mut self,
        data: &CustomData,
        policy: &HudPolicy,
    ) -> (Vec<HudPacket>, Vec<String>) {
        self.reconcile(data, policy)
    }
    fn reconcile(
        &mut self,
        data: &CustomData,
        policy: &HudPolicy,
    ) -> (Vec<HudPacket>, Vec<String>) {
        let revision = (data.revision(), super::catalog::revision());
        if self.rendered_revision == Some(revision) {
            return (Vec::new(), self.composition.errors.clone());
        }
        self.rendered_revision = Some(revision);
        let desired = self.composition.render(data, policy);
        let mut packets = Vec::new();
        let mut errors = Vec::new();
        match desired.sidebar {
            Ok(Some(sidebar)) => packets.extend(self.set_sidebar(sidebar.title, &sidebar.lines)),
            Ok(None) => packets.extend(self.clear_sidebar()),
            Err(error) => errors.push(error),
        }
        match desired.tab {
            Ok(pair) => {
                if self.tab_list.as_ref() != Some(&pair) {
                    packets.push(HudPacket::Tab(Box::new((pair.0.clone(), pair.1.clone()))));
                    self.tab_list = Some(pair);
                }
            }
            Err(error) => errors.push(error),
        }
        let removed: Vec<_> = self
            .boss_bars
            .keys()
            .filter(|id| !desired.bars.contains_key(id))
            .copied()
            .collect();
        for id in removed {
            packets.extend(self.remove_boss_bar(id));
        }
        for (id, bar) in desired.bars {
            match bar {
                Ok(Some(bar)) => packets.extend(self.set_boss_bar(id, bar)),
                Ok(None) => packets.extend(self.remove_boss_bar(id)),
                Err(error) => errors.push(error),
            }
        }
        if self.composition.errors != errors {
            for error in &errors {
                log::error!("HUD: {error}");
            }
            self.composition.errors.clone_from(&errors);
        }
        (packets, errors)
    }
}

impl Player {
    /// Last reconciliation errors for this connection.
    #[must_use]
    pub fn hud_conflicts(&self) -> Vec<String> {
        self.session.hud.lock().composition.errors.clone()
    }
    /// Renders owned contributions from current-domain data; failed surfaces keep their last valid state.
    pub fn refresh_hud(&self) -> Result<(), String> {
        let _dispatch = self.session.hud_dispatch.lock();
        let (packets, errors) = {
            let data = self.data();
            self.session.hud.lock().reconcile(&data, &self.config.hud)
        };
        self.send_hud(packets);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// Replaces only this source's sidebar section.
    pub fn set_hud_sidebar(
        &self,
        source: &Arc<HudSource>,
        section: Option<SidebarSection>,
    ) -> Result<(), String> {
        {
            let mut hud = self.session.hud.lock();
            hud.rendered_revision = None;
            hud.composition
                .sources
                .entry(source.key.clone())
                .or_default()
                .sidebar = section;
        }
        self.refresh_hud()
    }

    /// Replaces only this source's tab section.
    pub fn set_hud_tab(
        &self,
        source: &Arc<HudSource>,
        section: Option<TabSection>,
    ) -> Result<(), String> {
        {
            let mut hud = self.session.hud.lock();
            hud.rendered_revision = None;
            hud.composition
                .sources
                .entry(source.key.clone())
                .or_default()
                .tab = section;
        }
        self.refresh_hud()
    }

    /// Publishes a bar under a source-local UUID, isolated from identical UUIDs from other sources.
    pub fn set_hud_bar(
        &self,
        source: &Arc<HudSource>,
        local: Uuid,
        bar: Option<BarContribution>,
    ) -> Result<(), String> {
        if let Some(bar) = &bar
            && (!bar.bar.progress.is_finite() || bar.bar.flags & !7 != 0)
        {
            return Err("boss bar needs finite progress and valid flags".into());
        }
        {
            let mut hud = self.session.hud.lock();
            hud.rendered_revision = None;
            let bars = &mut hud
                .composition
                .sources
                .entry(source.key.clone())
                .or_default()
                .bars;
            match bar {
                Some(contribution) => {
                    if let Some(previous) = bars.get_mut(&local) {
                        previous.contribution = contribution;
                    } else {
                        bars.insert(
                            local,
                            OwnedBar {
                                id: Uuid::new_v4(),
                                contribution,
                            },
                        );
                    }
                }
                None => {
                    bars.remove(&local);
                }
            }
        }
        self.refresh_hud()
    }

    /// Removes all resources belonging to one source.
    pub fn clear_hud_source(&self, source: &Arc<HudSource>) -> Result<(), String> {
        {
            let mut hud = self.session.hud.lock();
            hud.rendered_revision = None;
            hud.composition.sources.remove(&source.key);
        }
        self.refresh_hud()
    }

    /// Updates the server's default tab contribution alongside extensions.
    pub fn set_server_tab(&self, header: TextComponent, footer: TextComponent) {
        {
            let mut hud = self.session.hud.lock();
            hud.rendered_revision = None;
            hud.composition.server_tab = Some((header, footer));
        }
        // Reconciliation already reports changed errors once to the owner.
        let _ = self.refresh_hud();
    }
}
