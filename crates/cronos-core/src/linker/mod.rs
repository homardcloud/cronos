use crate::graph::ContextGraph;
use crate::storage::Repository;
use cronos_model::*;

pub struct Linker {
    #[allow(dead_code)]
    temporal_window_ms: u64,
}

impl Linker {
    pub fn new(temporal_window_ms: u64) -> Self {
        Self { temporal_window_ms }
    }

    pub fn resolve_entity_ref(
        &self,
        entity_ref: &EntityRef,
        timestamp: Timestamp,
        repo: &Repository,
        graph: &mut ContextGraph,
    ) -> rusqlite::Result<Entity> {
        if let Some(mut existing) =
            repo.find_entity_by_kind_and_name(&entity_ref.kind, &entity_ref.identity)?
        {
            existing.last_seen = timestamp;
            repo.insert_entity(&existing)?;
            graph.add_entity(existing.id);
            return Ok(existing);
        }
        let entity = Entity {
            id: EntityId::new(),
            kind: entity_ref.kind.clone(),
            name: entity_ref.identity.clone(),
            attributes: entity_ref.attributes.clone(),
            first_seen: timestamp,
            last_seen: timestamp,
        };
        repo.insert_entity(&entity)?;
        graph.add_entity(entity.id);
        Ok(entity)
    }

    pub fn link(
        &self,
        event: &Event,
        repo: &Repository,
        graph: &mut ContextGraph,
    ) -> rusqlite::Result<()> {
        let now = event.timestamp;
        let subject = self.resolve_entity_ref(&event.subject, now, repo, graph)?;
        let mut context_ids = Vec::new();
        for ctx_ref in &event.context {
            let ctx_entity = self.resolve_entity_ref(ctx_ref, now, repo, graph)?;
            context_ids.push(ctx_entity.id);
            let relation = infer_relation(&event.subject.kind, &ctx_ref.kind);
            self.ensure_edge(subject.id, ctx_entity.id, relation, now, repo, graph)?;
        }
        repo.insert_event(event, subject.id, &context_ids)?;
        Ok(())
    }

    fn ensure_edge(
        &self,
        from: EntityId,
        to: EntityId,
        relation: Relation,
        timestamp: Timestamp,
        repo: &Repository,
        graph: &mut ContextGraph,
    ) -> rusqlite::Result<()> {
        if let Some(mut existing) = repo.find_edge(from, to, &relation)? {
            existing.strength = (existing.strength + 0.1).min(1.0);
            existing.last_reinforced = timestamp;
            repo.insert_edge(&existing)?;
            graph.add_edge(&existing);
        } else {
            let edge = Edge {
                id: EdgeId::new(),
                from,
                to,
                relation,
                strength: 0.5,
                created_at: timestamp,
                last_reinforced: timestamp,
            };
            repo.insert_edge(&edge)?;
            graph.add_edge(&edge);
        }
        Ok(())
    }
}

fn infer_relation(subject_kind: &EntityKind, context_kind: &EntityKind) -> Relation {
    match (subject_kind, context_kind) {
        (EntityKind::File, EntityKind::Project) => Relation::BelongsTo,
        (EntityKind::Commit, EntityKind::Repository) => Relation::BelongsTo,
        (EntityKind::Branch, EntityKind::Repository) => Relation::BelongsTo,
        (EntityKind::Url, EntityKind::Domain) => Relation::BelongsTo,
        (EntityKind::Project, EntityKind::Repository) => Relation::Contains,
        _ => Relation::RelatedTo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_entity_ref(kind: EntityKind, identity: &str) -> EntityRef {
        EntityRef {
            kind,
            identity: identity.to_string(),
            attributes: HashMap::new(),
        }
    }

    fn make_event_with_context(
        subject: EntityRef,
        context: Vec<EntityRef>,
        timestamp: Timestamp,
    ) -> Event {
        Event {
            id: EventId::new(),
            timestamp,
            source: CollectorSource::Filesystem,
            kind: EventKind::FileModified,
            subject,
            context,
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn resolve_creates_new_entity() {
        let repo = Repository::open_in_memory().unwrap();
        let mut graph = ContextGraph::new();
        let linker = Linker::new(5000);

        let entity_ref = make_entity_ref(EntityKind::File, "/src/main.rs");
        let entity = linker
            .resolve_entity_ref(&entity_ref, 1000, &repo, &mut graph)
            .unwrap();

        assert_eq!(entity.kind, EntityKind::File);
        assert_eq!(entity.name, "/src/main.rs");
        assert_eq!(entity.first_seen, 1000);
        assert_eq!(entity.last_seen, 1000);
        assert!(graph.has_entity(&entity.id));
        assert_eq!(repo.entity_count().unwrap(), 1);
    }

    #[test]
    fn resolve_finds_existing_entity() {
        let repo = Repository::open_in_memory().unwrap();
        let mut graph = ContextGraph::new();
        let linker = Linker::new(5000);

        let entity_ref = make_entity_ref(EntityKind::File, "/src/main.rs");
        let first = linker
            .resolve_entity_ref(&entity_ref, 1000, &repo, &mut graph)
            .unwrap();
        let second = linker
            .resolve_entity_ref(&entity_ref, 2000, &repo, &mut graph)
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.last_seen, 2000);
        // Still only one entity in the repo
        assert_eq!(repo.entity_count().unwrap(), 1);
    }

    #[test]
    fn link_creates_entities_and_edges() {
        let repo = Repository::open_in_memory().unwrap();
        let mut graph = ContextGraph::new();
        let linker = Linker::new(5000);

        let subject = make_entity_ref(EntityKind::File, "/src/main.rs");
        let context = vec![make_entity_ref(EntityKind::Project, "my-project")];
        let event = make_event_with_context(subject, context, 1000);

        linker.link(&event, &repo, &mut graph).unwrap();

        assert_eq!(repo.entity_count().unwrap(), 2);
        assert_eq!(repo.event_count().unwrap(), 1);
        assert_eq!(repo.edge_count().unwrap(), 1);
        assert_eq!(graph.entity_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn repeated_events_reinforce_edges() {
        let repo = Repository::open_in_memory().unwrap();
        let mut graph = ContextGraph::new();
        let linker = Linker::new(5000);

        let subject = make_entity_ref(EntityKind::File, "/src/main.rs");
        let context = vec![make_entity_ref(EntityKind::Project, "my-project")];

        let event1 = make_event_with_context(subject.clone(), context.clone(), 1000);
        linker.link(&event1, &repo, &mut graph).unwrap();

        let event2 = make_event_with_context(subject, context, 2000);
        linker.link(&event2, &repo, &mut graph).unwrap();

        let edges = repo.all_edges().unwrap();
        assert_eq!(edges.len(), 1);
        assert!(
            edges[0].strength > 0.5,
            "edge strength should be reinforced above 0.5, got {}",
            edges[0].strength
        );
    }

    #[test]
    fn infer_relation_file_to_project() {
        let relation = infer_relation(&EntityKind::File, &EntityKind::Project);
        assert_eq!(relation, Relation::BelongsTo);
    }
}
