//! Desired contributions stay independent of the last successfully rendered HUD.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use steel_utils::text::DisplayResolutor;
use text_components::{Modifier, TextComponent};
use uuid::Uuid;

use super::{BossBar, MAX_SIDEBAR_LINES, Sidebar, catalog::SourceKey, expression::RenderContext};
use crate::entity::custom_data::CustomData;

/// Owner-controlled HUD layout. Keys are `extension:source` names.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HudPolicy {
    /// Alias overrides pointing to namespaced player-data paths.
    pub aliases: BTreeMap<String, String>,
    /// Sources to render first; remaining sources sort by their full key.
    pub order: Vec<String>,
    /// Sources suppressed on all surfaces.
    pub hidden: BTreeSet<String>,
    /// Replaces the combined default sidebar title.
    pub sidebar_title: Option<String>,
    /// Replaces sidebar sections with an explicit template layout.
    pub sidebar_lines: Option<Vec<String>>,
    /// Replaces the combined tab header.
    pub tab_header: Option<String>,
    /// Replaces the combined tab footer.
    pub tab_footer: Option<String>,
    /// Overrides individual bars, keyed by `extension:source/local-uuid`.
    pub boss_bars: BTreeMap<String, BossBarPolicy>,
}

/// A data-driven replacement for an extension's default boss bar.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct BossBarPolicy {
    /// Suppresses this bar without removing its producer.
    pub hidden: bool,
    /// Title template.
    pub title: Option<String>,
    /// Alias or canonical path of a gauge containing `current` and `maximum`.
    pub gauge: Option<String>,
}

/// One source's default sidebar section.
#[derive(Debug, Clone)]
pub struct SidebarSection {
    /// Rich-text or plain-text title template. Empty titles keep lines without a header label.
    pub title: String,
    /// Rich-text or plain-text line templates, in display order.
    pub lines: Vec<String>,
}

/// One source's default tab header and footer templates.
#[derive(Debug, Clone)]
pub struct TabSection {
    /// Header template.
    pub header: String,
    /// Footer template.
    pub footer: String,
}

/// Boss-bar defaults, optionally driven by a player-data gauge.
#[derive(Debug, Clone)]
pub struct BarContribution {
    /// Appearance and fallback numeric progress.
    pub bar: BossBar,
    /// Unrendered title template.
    pub title: String,
    /// Gauge reference for progress, if data-driven.
    pub gauge: Option<String>,
}

#[derive(Debug)]
pub(super) struct OwnedBar {
    pub id: Uuid,
    pub contribution: BarContribution,
}

/// Owned resources submitted by one producer.
#[derive(Debug, Default)]
pub(super) struct Contributions {
    pub sidebar: Option<SidebarSection>,
    pub tab: Option<TabSection>,
    pub bars: BTreeMap<Uuid, OwnedBar>,
}

#[derive(Debug, Default)]
pub(super) struct Composition {
    pub sources: BTreeMap<SourceKey, Contributions>,
    pub server_tab: Option<(TextComponent, TextComponent)>,
    pub errors: Vec<String>,
}

pub(super) struct Rendered {
    pub sidebar: Result<Option<Sidebar>, String>,
    pub tab: Result<(TextComponent, TextComponent), String>,
    pub bars: BTreeMap<Uuid, Result<Option<BossBar>, String>>,
}

fn join(parts: Vec<TextComponent>, separator: &str) -> TextComponent {
    let mut children = Vec::new();
    for part in parts {
        if !children.is_empty() {
            children.push(TextComponent::plain(separator.to_owned()));
        }
        children.push(part);
    }
    TextComponent::plain("").add_children(children)
}

impl Composition {
    pub fn render(&self, data: &CustomData, policy: &HudPolicy) -> Rendered {
        let mut sources: Vec<_> = self
            .sources
            .iter()
            .filter(|(key, _)| !policy.hidden.contains(&key.0))
            .collect();
        sources.sort_by_key(|(key, _)| {
            (
                policy
                    .order
                    .iter()
                    .position(|s| s == &key.0)
                    .unwrap_or(usize::MAX),
                &key.0,
            )
        });
        let sidebar = Self::render_sidebar(data, policy, &sources);
        let tab = self.render_tab(data, policy, &sources);
        let mut bars = BTreeMap::new();
        for (owner, claims) in &sources {
            for (local, entry) in &claims.bars {
                let render = (|| {
                    let key = format!("{}/{local}", owner.0);
                    let settings = policy.boss_bars.get(&key);
                    if settings.is_some_and(|settings| settings.hidden) {
                        return Ok(None);
                    }
                    let ctx = RenderContext {
                        data,
                        owner: Some(*owner),
                        overrides: &policy.aliases,
                    };
                    let defaults = &entry.contribution;
                    let title = settings
                        .and_then(|s| s.title.as_ref())
                        .unwrap_or(&defaults.title);
                    let Some(title) = ctx.text(title)? else {
                        return Ok(None);
                    };
                    let mut bar = defaults.bar.clone();
                    bar.title = title;
                    if let Some(gauge) = settings
                        .and_then(|s| s.gauge.as_ref())
                        .or(defaults.gauge.as_ref())
                    {
                        let Some(progress) = ctx.ratio(gauge)? else {
                            return Ok(None);
                        };
                        bar.progress = progress;
                    }
                    Ok(Some(bar))
                })();
                bars.insert(entry.id, render);
            }
        }
        Rendered { sidebar, tab, bars }
    }
    fn render_sidebar(
        data: &CustomData,
        policy: &HudPolicy,
        sources: &[(&SourceKey, &Contributions)],
    ) -> Result<Option<Sidebar>, String> {
        let context = |owner| RenderContext {
            data,
            owner,
            overrides: &policy.aliases,
        };
        let mut titles = Vec::new();
        let mut lines = Vec::new();
        let mut owners = Vec::new();
        if let Some(layout) = &policy.sidebar_lines {
            for line in layout {
                if let Some(line) = context(None).text(line)? {
                    lines.push(line);
                }
            }
        } else {
            for (owner, claims) in sources {
                let Some(section) = &claims.sidebar else {
                    continue;
                };
                let ctx = context(Some(*owner));
                let Some(title) = ctx.text(&section.title)? else {
                    continue;
                };
                let mut section_lines = Vec::new();
                for line in &section.lines {
                    if let Some(line) = ctx.text(line)? {
                        section_lines.push(line);
                    }
                }
                if section_lines.is_empty() {
                    continue;
                }
                owners.push(owner.0.as_str());
                if !title.to_plain(&DisplayResolutor).is_empty() {
                    titles.push(title);
                }
                lines.extend(section_lines);
            }
        }
        if lines.len() > MAX_SIDEBAR_LINES {
            return Err(format!(
                "sidebar has {} lines from [{}]; limit is {MAX_SIDEBAR_LINES}; configure server.hud.order, hidden, or sidebar_lines",
                lines.len(),
                owners.join(", ")
            ));
        }
        if lines.is_empty() {
            return Ok(None);
        }
        let title = match &policy.sidebar_title {
            Some(template) => context(None)
                .text(template)?
                .unwrap_or_else(|| TextComponent::plain("")),
            None => join(titles, " | "),
        };
        Ok(Some(Sidebar { title, lines }))
    }

    fn render_tab(
        &self,
        data: &CustomData,
        policy: &HudPolicy,
        sources: &[(&SourceKey, &Contributions)],
    ) -> Result<(TextComponent, TextComponent), String> {
        let context = |owner| RenderContext {
            data,
            owner,
            overrides: &policy.aliases,
        };
        let mut headers = Vec::new();
        let mut footers = Vec::new();
        if !policy.hidden.contains("steel:server")
            && let Some((header, footer)) = &self.server_tab
        {
            headers.push(header.clone());
            footers.push(footer.clone());
        }
        for (owner, claims) in sources {
            let Some(section) = &claims.tab else {
                continue;
            };
            let ctx = context(Some(*owner));
            if let Some(header) = ctx.text(&section.header)? {
                headers.push(header);
            }
            if let Some(footer) = ctx.text(&section.footer)? {
                footers.push(footer);
            }
        }
        let header = match &policy.tab_header {
            Some(template) => context(None)
                .text(template)?
                .unwrap_or_else(|| TextComponent::plain("")),
            None => join(headers, "\n"),
        };
        let footer = match &policy.tab_footer {
            Some(template) => context(None)
                .text(template)?
                .unwrap_or_else(|| TextComponent::plain("")),
            None => join(footers, "\n"),
        };
        Ok((header, footer))
    }
}
