//! Domain-aware loaded worlds. Readers own snapshots so worlds may retire at tick boundaries.
use crate::config::{ResolvedDomainConfig, ResolvedWorldConfig};
use crate::world::{DomainEntityDirectory, World};
use rustc_hash::{FxHashMap, FxHashSet};
use small_map::FxSmallMap;
use std::sync::Arc;
use steel_utils::{Identifier, locks::SyncRwLock};

pub(crate) const OVERWORLD_WORLD_NAME: &str = "overworld";
pub(crate) const NETHER_WORLD_NAME: &str = "the_nether";
pub(crate) const END_WORLD_NAME: &str = "the_end";

/// Loaded worlds plus domain defaults. Mutations are published at server tick boundaries.
pub struct WorldMap {
    default_domain: String,
    state: SyncRwLock<WorldMapState>,
}
/// Where a runtime world joins the map.
pub(crate) enum RuntimePlacement {
    /// First world of a new temporary domain; it becomes the domain's default world.
    NewDomainDefault,
    /// Additional world of a loaded domain.
    Member,
}
/// A loaded domain. The default world is `None` once a temporary domain's first world is removed.
struct DomainEntry {
    default_world: Option<Identifier>,
}
struct WorldMapState {
    worlds: FxSmallMap<8, Identifier, Arc<World>>,
    domains: FxHashMap<String, DomainEntry>,
    /// Worlds created at runtime, the only ones that can be removed.
    runtime_worlds: FxHashSet<Identifier>,
    /// Worlds whose game-time clock is advanced each tick. Derived worlds share another's clock.
    clock_owners: Vec<Identifier>,
    nether_portal_targets: FxHashMap<Identifier, Identifier>,
    end_portal_targets: FxHashMap<Identifier, Identifier>,
    entity_directories: FxHashMap<String, Arc<DomainEntityDirectory>>,
}
impl WorldMap {
    /// Creates a world map from resolved domain config.
    #[must_use]
    pub fn new(
        default_domain: String,
        domains: &[ResolvedDomainConfig],
        configs: &[ResolvedWorldConfig],
    ) -> Self {
        Self {
            default_domain,
            state: SyncRwLock::new(WorldMapState {
                worlds: FxSmallMap::default(),
                domains: domains
                    .iter()
                    .map(|d| {
                        (
                            d.name.clone(),
                            DomainEntry {
                                default_world: Some(d.default_world.clone()),
                            },
                        )
                    })
                    .collect(),
                runtime_worlds: FxHashSet::default(),
                clock_owners: domains.iter().map(|d| d.default_world.clone()).collect(),
                nether_portal_targets: configs
                    .iter()
                    .filter_map(|w| {
                        w.nether_portal_target
                            .as_ref()
                            .map(|t| (w.key.clone(), t.clone()))
                    })
                    .collect(),
                end_portal_targets: configs
                    .iter()
                    .filter_map(|w| {
                        w.end_portal_target
                            .as_ref()
                            .map(|t| (w.key.clone(), t.clone()))
                    })
                    .collect(),
                entity_directories: FxHashMap::default(),
            }),
        }
    }
    pub(crate) fn validate_game_times(&self) -> Result<(), String> {
        for domain in self.domain_names() {
            let primary = self
                .default_world(&domain)
                .ok_or_else(|| format!("domain {domain} has no loaded primary"))?;
            if primary.domain() != domain || !primary.level_data.read().owns_game_time() {
                return Err(format!("domain {domain} has an invalid game-time primary"));
            }
            for world in self.values().filter(|world| world.domain() == domain) {
                if !Arc::ptr_eq(&primary.game_time, &world.game_time)
                    || (world.key != primary.key && world.level_data.read().owns_game_time())
                {
                    return Err(format!(
                        "world {} is not bound to domain {domain}'s primary clock",
                        world.key
                    ));
                }
            }
        }
        for world in self.values() {
            if !self.has_domain(world.domain()) {
                return Err(format!("world {} has no configured domain", world.key));
            }
        }
        Ok(())
    }
    /// Advances each domain's shared simulation clock before worker dispatch.
    ///
    /// # Panics
    /// Panics if a validated domain primary is missing.
    pub fn advance_domain_game_times(&self) {
        let state = self.state.read();
        for key in &state.clock_owners {
            let Some(owner) = state.worlds.get(key) else {
                panic!("validated clock owner is missing: {key}");
            };
            owner.level_data.write().advance_game_time();
        }
    }
    /// Inserts a loaded world during initial construction.
    pub fn insert(&mut self, key: Identifier, world: Arc<World>) {
        let mut state = self.state.write();
        Self::insert_into(&mut state, key, world);
    }
    fn insert_into(state: &mut WorldMapState, key: Identifier, world: Arc<World>) {
        if let Some(replaced) = state.worlds.insert(key, Arc::clone(&world))
            && let Some(directory) = state.entity_directories.get(replaced.domain())
        {
            directory.remove_world(&replaced);
        }
        let directory = state
            .entity_directories
            .entry(world.domain().to_owned())
            .or_insert_with(|| Arc::new(DomainEntityDirectory::new()));
        directory.add_world(&world);
        world.set_domain_entity_directory(Arc::clone(directory));
    }
    /// Publishes a runtime world at a tick boundary.
    ///
    /// A member that owns its clock is advanced on its own; a derived member must share its
    /// domain default world's clock.
    pub(crate) fn publish_runtime_world(
        &self,
        world: Arc<World>,
        placement: RuntimePlacement,
    ) -> Result<(), String> {
        let mut state = self.state.write();
        if state.worlds.get(&world.key).is_some() {
            return Err(format!("world {} already exists", world.key));
        }
        let owns_clock = world.level_data.read().owns_game_time();
        match placement {
            RuntimePlacement::NewDomainDefault => {
                if state.domains.contains_key(world.domain()) {
                    return Err(format!("domain {} already exists", world.domain()));
                }
                if !owns_clock {
                    return Err("temporary primary must own its clock".into());
                }
                state.domains.insert(
                    world.domain().to_owned(),
                    DomainEntry {
                        default_world: Some(world.key.clone()),
                    },
                );
            }
            RuntimePlacement::Member if !owns_clock => {
                let entry = state
                    .domains
                    .get(world.domain())
                    .ok_or("domain no longer exists")?;
                let key = entry
                    .default_world
                    .as_ref()
                    .ok_or("domain has no default world to share a clock with")?;
                let parent = state.worlds.get(key).ok_or("domain primary is missing")?;
                if !Arc::ptr_eq(&parent.game_time, &world.game_time) {
                    return Err("temporary world must share its domain clock".into());
                }
            }
            RuntimePlacement::Member => {
                if !state.domains.contains_key(world.domain()) {
                    return Err("domain no longer exists".into());
                }
            }
        }
        if owns_clock {
            state.clock_owners.push(world.key.clone());
        }
        state.runtime_worlds.insert(world.key.clone());
        Self::insert_into(&mut state, world.key.clone(), world);
        Ok(())
    }
    pub(crate) fn remove_temporary_domain(&self, domain: &str) -> Vec<Arc<World>> {
        let mut state = self.state.write();
        let keys: Vec<_> = state
            .worlds
            .iter()
            .filter(|(_, w)| w.domain() == domain)
            .map(|(k, _)| k.clone())
            .collect();
        let mut removed = Vec::new();
        for key in keys {
            if let Some(world) = state.worlds.remove(&key) {
                if let Some(directory) = state.entity_directories.get(domain) {
                    directory.remove_world(&world);
                }
                removed.push(world);
            }
            state.nether_portal_targets.remove(&key);
            state.end_portal_targets.remove(&key);
            state.runtime_worlds.remove(&key);
            state.clock_owners.retain(|owner| *owner != key);
        }
        state.domains.remove(domain);
        state.entity_directories.remove(domain);
        removed
    }
    /// Returns an owned reference, valid even if the world retires later.
    #[must_use]
    pub fn get(&self, key: &Identifier) -> Option<Arc<World>> {
        self.state.read().worlds.get(key).map(Arc::clone)
    }
    /// Takes a snapshot of loaded worlds without retaining a map lock.
    pub fn values(&self) -> impl Iterator<Item = Arc<World>> {
        self.state
            .read()
            .worlds
            .values()
            .map(Arc::clone)
            .collect::<Vec<_>>()
            .into_iter()
    }
    /// Takes a snapshot of loaded world keys.
    pub fn keys(&self) -> impl Iterator<Item = Identifier> {
        self.state
            .read()
            .worlds
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
    }
    /// Takes a snapshot of loaded world entries.
    pub fn iter(&self) -> impl Iterator<Item = (Identifier, Arc<World>)> {
        self.state
            .read()
            .worlds
            .iter()
            .map(|(k, w)| (k.clone(), Arc::clone(w)))
            .collect::<Vec<_>>()
            .into_iter()
    }
    /// Number of loaded worlds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.read().worlds.len()
    }
    /// Whether any world is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state.read().worlds.is_empty()
    }
    /// Server's permanent default domain.
    #[must_use]
    pub fn default_domain(&self) -> &str {
        &self.default_domain
    }
    /// Whether the domain is published.
    #[must_use]
    pub fn has_domain(&self, domain: &str) -> bool {
        self.state.read().domains.contains_key(domain)
    }
    /// Takes a snapshot of loaded domain names.
    pub fn domain_names(&self) -> impl Iterator<Item = String> {
        self.state
            .read()
            .domains
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
    }
    /// Takes a snapshot of every domain's default world. Domain-level saved data (scoreboard,
    /// command storage) is written through these worlds only; other worlds, including runtime
    /// worlds that own their clock, never consume it.
    #[must_use]
    pub(crate) fn domain_default_worlds(&self) -> Vec<Arc<World>> {
        let state = self.state.read();
        state
            .domains
            .values()
            .filter_map(|entry| {
                let key = entry.default_world.as_ref()?;
                state.worlds.get(key).map(Arc::clone)
            })
            .collect()
    }
    /// Returns a domain's primary world.
    #[must_use]
    pub fn default_world(&self, domain: &str) -> Option<Arc<World>> {
        let state = self.state.read();
        let key = state.domains.get(domain)?.default_world.as_ref()?;
        state.worlds.get(key).map(Arc::clone)
    }
    /// Returns the server's permanent default world.
    #[must_use]
    pub fn server_default_world(&self) -> Option<Arc<World>> {
        self.default_world(self.default_domain())
    }
    /// Takes a snapshot of worlds in one domain.
    #[must_use]
    pub fn worlds_in_domain(&self, domain: &str) -> Vec<Arc<World>> {
        self.values().filter(|w| w.domain() == domain).collect()
    }
    /// Resolves a conventional portal world in the source domain.
    #[must_use]
    pub fn resolve_portal_target(&self, source: &World, name: &str) -> Option<Arc<World>> {
        self.get(&Identifier::new(
            source.domain().to_owned(),
            name.to_owned(),
        ))
    }
    /// Resolves a Nether portal destination.
    #[must_use]
    pub fn resolve_nether_portal_target(&self, source: &World) -> Option<Arc<World>> {
        let target = self
            .state
            .read()
            .nether_portal_targets
            .get(&source.key)
            .cloned();
        if let Some(target) = target {
            return self.get(&target);
        }
        self.resolve_portal_target(
            source,
            nether_portal_target_world_name(source.key.path.as_ref()),
        )
    }
    /// Resolves an End entry portal destination.
    #[must_use]
    pub fn resolve_end_entry_portal_target(&self, source: &World) -> Option<Arc<World>> {
        let target = self
            .state
            .read()
            .end_portal_targets
            .get(&source.key)
            .cloned();
        if let Some(target) = target {
            return self.get(&target);
        }
        end_entry_portal_target_world_name(source.key.path.as_ref())
            .and_then(|name| self.resolve_portal_target(source, name))
    }
}
fn nether_portal_target_world_name(source: &str) -> &'static str {
    if source == NETHER_WORLD_NAME {
        OVERWORLD_WORLD_NAME
    } else {
        NETHER_WORLD_NAME
    }
}
fn end_entry_portal_target_world_name(source: &str) -> Option<&'static str> {
    if source == END_WORLD_NAME {
        None
    } else {
        Some(END_WORLD_NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::{WorldMap, end_entry_portal_target_world_name, nether_portal_target_world_name};

    fn runtime_map(domain: &'static str) -> (WorldMap, crate::test_support::TestWorld) {
        use crate::config::ResolvedDomainConfig;
        use crate::test_support::fresh_test_world_in_domain;
        use steel_utils::Identifier;

        let default = fresh_test_world_in_domain(domain, "overworld");
        let config = ResolvedDomainConfig {
            name: domain.to_owned(),
            default_world: default.world.key.clone(),
            worlds: vec![default.world.key.clone()],
        };
        let mut worlds = WorldMap::new(domain.to_owned(), &[config], &[]);
        worlds.insert(
            Identifier::new_static(domain, "overworld"),
            default.world.clone(),
        );
        (worlds, default)
    }

    #[test]
    fn runtime_member_with_its_own_clock_advances_independently() {
        use super::RuntimePlacement;
        use crate::test_support::fresh_test_world_in_domain;

        let (worlds, default) = runtime_map("own_clock");
        let member = fresh_test_world_in_domain("own_clock", "clone");
        worlds
            .publish_runtime_world(member.world.clone(), RuntimePlacement::Member)
            .expect("a member may own its clock");
        let member_before = member.world.game_time.ticks();
        let default_before = default.world.game_time.ticks();

        worlds.advance_domain_game_times();
        worlds.advance_domain_game_times();

        assert_eq!(member.world.game_time.ticks(), member_before + 2);
        assert_eq!(default.world.game_time.ticks(), default_before + 2);
        assert!(!std::sync::Arc::ptr_eq(
            &member.world.game_time,
            &default.world.game_time
        ));
    }

    #[test]
    fn domain_without_a_default_world_stays_registered() {
        use super::RuntimePlacement;
        use crate::test_support::fresh_test_world_in_domain;

        let (worlds, _default) = runtime_map("no_default");
        let member = fresh_test_world_in_domain("no_default", "clone");
        worlds
            .publish_runtime_world(member.world.clone(), RuntimePlacement::Member)
            .expect("member publishes");
        worlds
            .state
            .write()
            .domains
            .get_mut("no_default")
            .expect("domain")
            .default_world = None;

        worlds.advance_domain_game_times();

        assert!(worlds.has_domain("no_default"));
        assert!(worlds.default_world("no_default").is_none());
        assert!(worlds.domain_default_worlds().is_empty());
        assert_eq!(worlds.worlds_in_domain("no_default").len(), 2);
    }

    #[test]
    fn derived_members_must_share_the_domain_clock() {
        use super::RuntimePlacement;
        use crate::test_support::{fresh_test_derived_world, fresh_test_world_in_domain};

        let (worlds, default) = runtime_map("derived_clock");
        let sibling = fresh_test_derived_world(&default.world, "sibling");
        worlds
            .publish_runtime_world(sibling.world.clone(), RuntimePlacement::Member)
            .expect("a derived sibling shares the default clock");

        let other_primary = fresh_test_world_in_domain("derived_clock", "other_primary");
        let stranger = fresh_test_derived_world(&other_primary.world, "stranger");
        assert!(
            worlds
                .publish_runtime_world(stranger.world.clone(), RuntimePlacement::Member)
                .is_err(),
            "a derived world of another clock is refused"
        );
    }

    #[test]
    fn domain_saved_data_targets_only_default_worlds() {
        use crate::config::ResolvedDomainConfig;
        use crate::test_support::fresh_test_world_in_domain;
        use steel_utils::Identifier;

        let default = fresh_test_world_in_domain("saved_targets", "overworld");
        let clock_owner = fresh_test_world_in_domain("saved_targets", "clone");
        assert!(clock_owner.world.level_data.read().owns_game_time());
        let domain = ResolvedDomainConfig {
            name: "saved_targets".to_owned(),
            default_world: Identifier::new_static("saved_targets", "overworld"),
            worlds: vec![default.world.key.clone(), clock_owner.world.key.clone()],
        };
        let mut worlds = WorldMap::new("saved_targets".to_owned(), &[domain], &[]);
        worlds.insert(clock_owner.world.key.clone(), clock_owner.world.clone());
        worlds.insert(default.world.key.clone(), default.world.clone());

        let targets = worlds.domain_default_worlds();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].key, default.world.key);
    }

    #[test]
    fn domain_uuid_lookup_excludes_replaced_worlds_without_retaining_worlds() {
        use crate::entity::{SharedEntity, entities::PigEntity};
        use crate::test_support::{
            fresh_test_derived_world, fresh_test_world_in_domain, insert_ready_full_chunk,
        };
        use glam::DVec3;
        use std::sync::Arc;
        use steel_registry::vanilla_entities;
        use steel_utils::ChunkPos;

        let primary_fixture = fresh_test_world_in_domain("directory", "overworld");
        let primary = &primary_fixture.world;
        let other_fixture = fresh_test_derived_world(primary, "other");
        let other = &other_fixture.world;
        insert_ready_full_chunk(other, ChunkPos::new(0, 0));
        let owner: SharedEntity = Arc::new(PigEntity::new(
            &vanilla_entities::PIG,
            1,
            DVec3::ZERO,
            Arc::downgrade(other),
        ));
        other
            .try_add_entity(Arc::clone(&owner))
            .expect("register owner");
        let mut worlds = super::WorldMap::new("directory".to_owned(), &[], &[]);
        worlds.insert(primary.key.clone(), Arc::clone(primary));
        worlds.insert(other.key.clone(), Arc::clone(other));
        assert!(Arc::ptr_eq(
            &primary
                .get_entity_in_domain_by_uuid(&owner.uuid())
                .expect("other world owner"),
            &owner
        ));

        let replacement_fixture = fresh_test_derived_world(primary, "other");
        let replacement = &replacement_fixture.world;
        worlds.insert(replacement.key.clone(), Arc::clone(replacement));
        assert!(
            primary
                .get_entity_in_domain_by_uuid(&owner.uuid())
                .is_none()
        );
        assert!(
            other.get_entity_by_uuid(&owner.uuid()).is_some(),
            "old world is still alive but no longer loaded in the map"
        );
        let weak = Arc::downgrade(replacement);
        drop(replacement_fixture);
        drop(worlds);
        assert!(
            weak.upgrade().is_none(),
            "domain directories must not keep worlds alive"
        );
    }

    #[test]
    fn nether_portal_target_names_follow_vanilla_level_keys() {
        assert_eq!(nether_portal_target_world_name("overworld"), "the_nether");
        assert_eq!(nether_portal_target_world_name("the_end"), "the_nether");
        assert_eq!(nether_portal_target_world_name("the_nether"), "overworld");
    }

    #[test]
    fn end_entry_portal_target_name_is_only_for_non_end_sources() {
        assert_eq!(
            end_entry_portal_target_world_name("overworld"),
            Some("the_end")
        );
        assert_eq!(
            end_entry_portal_target_world_name("the_nether"),
            Some("the_end")
        );
        assert_eq!(end_entry_portal_target_world_name("the_end"), None);
    }
}
