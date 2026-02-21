use cronos_model::{
    Attributes, CollectorSource, Edge, EdgeId, Entity, EntityId, EntityKind, Event, EventId,
    EventKind, Relation, Timestamp,
};
use rusqlite::{params, Connection};

use super::migrations::run_migrations;

/// SQLite-backed repository for entities, events, and edges.
pub struct Repository {
    conn: Connection,
}

impl Repository {
    /// Open a repository backed by a file at `path`.
    pub fn open(path: &std::path::Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        run_migrations(&conn)?;
        Ok(Self { conn })
    }

    /// Open an in-memory repository (useful for tests).
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        run_migrations(&conn)?;
        Ok(Self { conn })
    }

    // ─── Entity operations ───────────────────────────────────────────

    /// Insert or update an entity.
    ///
    /// On conflict (same id), updates `last_seen`, `name`, and `attributes`.
    /// Also maintains the FTS index.
    pub fn insert_entity(&self, entity: &Entity) -> rusqlite::Result<()> {
        let kind_str = serde_json::to_string(&entity.kind).unwrap();
        let attrs_str = serde_json::to_string(&entity.attributes).unwrap();

        self.conn.execute(
            "INSERT INTO entities (id, kind, name, attributes, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                last_seen  = excluded.last_seen,
                name       = excluded.name,
                attributes = excluded.attributes",
            params![
                entity.id.to_string(),
                kind_str,
                entity.name,
                attrs_str,
                entity.first_seen,
                entity.last_seen,
            ],
        )?;

        // Update FTS index: delete old entry (if any) then insert
        // We use the rowid of the just-inserted/updated entity row
        let rowid: i64 = self.conn.query_row(
            "SELECT rowid FROM entities WHERE id = ?1",
            [entity.id.to_string()],
            |row| row.get(0),
        )?;

        self.conn.execute(
            "INSERT OR REPLACE INTO entities_fts (rowid, name, attributes) VALUES (?1, ?2, ?3)",
            params![rowid, entity.name, attrs_str],
        )?;

        Ok(())
    }

    /// Get an entity by its id.
    pub fn get_entity(&self, id: EntityId) -> rusqlite::Result<Option<Entity>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, name, attributes, first_seen, last_seen FROM entities WHERE id = ?1",
        )?;

        let mut rows = stmt.query_map([id.to_string()], |row| {
            Ok(EntityRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                name: row.get(2)?,
                attributes: row.get(3)?,
                first_seen: row.get(4)?,
                last_seen: row.get(5)?,
            })
        })?;

        match rows.next() {
            Some(Ok(r)) => Ok(Some(entity_from_row(r))),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    /// Find an entity by kind and name.
    pub fn find_entity_by_kind_and_name(
        &self,
        kind: &EntityKind,
        name: &str,
    ) -> rusqlite::Result<Option<Entity>> {
        let kind_str = serde_json::to_string(kind).unwrap();

        let mut stmt = self.conn.prepare(
            "SELECT id, kind, name, attributes, first_seen, last_seen
             FROM entities
             WHERE kind = ?1 AND name = ?2",
        )?;

        let mut rows = stmt.query_map(params![kind_str, name], |row| {
            Ok(EntityRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                name: row.get(2)?,
                attributes: row.get(3)?,
                first_seen: row.get(4)?,
                last_seen: row.get(5)?,
            })
        })?;

        match rows.next() {
            Some(Ok(r)) => Ok(Some(entity_from_row(r))),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    /// Return all entities.
    pub fn all_entities(&self) -> rusqlite::Result<Vec<Entity>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, name, attributes, first_seen, last_seen FROM entities",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(EntityRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                name: row.get(2)?,
                attributes: row.get(3)?,
                first_seen: row.get(4)?,
                last_seen: row.get(5)?,
            })
        })?;

        let mut entities = Vec::new();
        for r in rows {
            entities.push(entity_from_row(r?));
        }
        Ok(entities)
    }

    /// Return the total count of entities.
    pub fn entity_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
    }

    // ─── Event operations ────────────────────────────────────────────

    /// Insert an event with its resolved subject entity id and context entity ids.
    ///
    /// The subject entity and all context entities must already exist in the
    /// entities table.
    pub fn insert_event(
        &self,
        event: &Event,
        subject_entity_id: EntityId,
        context_entity_ids: &[EntityId],
    ) -> rusqlite::Result<()> {
        let source_str = serde_json::to_string(&event.source).unwrap();
        let kind_str = serde_json::to_string(&event.kind).unwrap();
        let metadata_str = serde_json::to_string(&event.metadata).unwrap();

        self.conn.execute(
            "INSERT INTO events (id, timestamp, source, kind, subject_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.id.to_string(),
                event.timestamp,
                source_str,
                kind_str,
                subject_entity_id.to_string(),
                metadata_str,
            ],
        )?;

        // Insert context entity references
        let mut stmt = self.conn.prepare(
            "INSERT INTO event_context (event_id, entity_id) VALUES (?1, ?2)",
        )?;
        for ctx_id in context_entity_ids {
            stmt.execute(params![event.id.to_string(), ctx_id.to_string()])?;
        }

        Ok(())
    }

    /// Return events within the given timestamp range, ordered by timestamp ascending.
    pub fn events_in_range(
        &self,
        start: Timestamp,
        end: Timestamp,
    ) -> rusqlite::Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, source, kind, subject_id, metadata
             FROM events
             WHERE timestamp >= ?1 AND timestamp <= ?2
             ORDER BY timestamp ASC",
        )?;

        let rows = stmt.query_map(params![start, end], |row| {
            Ok(EventRow {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                source: row.get(2)?,
                kind: row.get(3)?,
                subject_id: row.get(4)?,
                metadata: row.get(5)?,
            })
        })?;

        let mut events = Vec::new();
        for r in rows {
            events.push(stored_event_from_row(r?));
        }
        Ok(events)
    }

    /// Return the most recent events, ordered by timestamp descending.
    pub fn recent_events(&self, limit: u32) -> rusqlite::Result<Vec<StoredEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, source, kind, subject_id, metadata
             FROM events
             ORDER BY timestamp DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map([limit], |row| {
            Ok(EventRow {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                source: row.get(2)?,
                kind: row.get(3)?,
                subject_id: row.get(4)?,
                metadata: row.get(5)?,
            })
        })?;

        let mut events = Vec::new();
        for r in rows {
            events.push(stored_event_from_row(r?));
        }
        Ok(events)
    }

    /// Return the total count of events.
    pub fn event_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
    }

    // ─── Edge operations ─────────────────────────────────────────────

    /// Insert or update an edge.
    ///
    /// On conflict (same id), updates `strength` and `last_reinforced`.
    pub fn insert_edge(&self, edge: &Edge) -> rusqlite::Result<()> {
        let relation_str = serde_json::to_string(&edge.relation).unwrap();

        self.conn.execute(
            "INSERT INTO edges (id, from_id, to_id, relation, strength, created_at, last_reinforced)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                strength        = excluded.strength,
                last_reinforced = excluded.last_reinforced",
            params![
                edge.id.to_string(),
                edge.from.to_string(),
                edge.to.to_string(),
                relation_str,
                edge.strength,
                edge.created_at,
                edge.last_reinforced,
            ],
        )?;

        Ok(())
    }

    /// Find an edge by from entity, to entity, and relation.
    pub fn find_edge(
        &self,
        from: EntityId,
        to: EntityId,
        relation: &Relation,
    ) -> rusqlite::Result<Option<Edge>> {
        let relation_str = serde_json::to_string(relation).unwrap();

        let mut stmt = self.conn.prepare(
            "SELECT id, from_id, to_id, relation, strength, created_at, last_reinforced
             FROM edges
             WHERE from_id = ?1 AND to_id = ?2 AND relation = ?3",
        )?;

        let mut rows = stmt.query_map(
            params![from.to_string(), to.to_string(), relation_str],
            |row| {
                Ok(EdgeRow {
                    id: row.get(0)?,
                    from_id: row.get(1)?,
                    to_id: row.get(2)?,
                    relation: row.get(3)?,
                    strength: row.get(4)?,
                    created_at: row.get(5)?,
                    last_reinforced: row.get(6)?,
                })
            },
        )?;

        match rows.next() {
            Some(Ok(r)) => Ok(Some(edge_from_row(r))),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    /// Return all edges from a given entity.
    pub fn edges_from(&self, entity_id: EntityId) -> rusqlite::Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, from_id, to_id, relation, strength, created_at, last_reinforced
             FROM edges
             WHERE from_id = ?1",
        )?;

        let rows = stmt.query_map([entity_id.to_string()], |row| {
            Ok(EdgeRow {
                id: row.get(0)?,
                from_id: row.get(1)?,
                to_id: row.get(2)?,
                relation: row.get(3)?,
                strength: row.get(4)?,
                created_at: row.get(5)?,
                last_reinforced: row.get(6)?,
            })
        })?;

        let mut edges = Vec::new();
        for r in rows {
            edges.push(edge_from_row(r?));
        }
        Ok(edges)
    }

    /// Return all edges.
    pub fn all_edges(&self) -> rusqlite::Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, from_id, to_id, relation, strength, created_at, last_reinforced FROM edges",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(EdgeRow {
                id: row.get(0)?,
                from_id: row.get(1)?,
                to_id: row.get(2)?,
                relation: row.get(3)?,
                strength: row.get(4)?,
                created_at: row.get(5)?,
                last_reinforced: row.get(6)?,
            })
        })?;

        let mut edges = Vec::new();
        for r in rows {
            edges.push(edge_from_row(r?));
        }
        Ok(edges)
    }

    /// Return the total count of edges.
    pub fn edge_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
    }

    // ─── Search ──────────────────────────────────────────────────────

    /// Full-text search over entity names and attributes.
    ///
    /// Returns matching entities up to `limit`.
    pub fn search_entities(&self, query: &str, limit: u32) -> rusqlite::Result<Vec<Entity>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.kind, e.name, e.attributes, e.first_seen, e.last_seen
             FROM entities_fts f
             JOIN entities e ON e.rowid = f.rowid
             WHERE entities_fts MATCH ?1
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![query, limit], |row| {
            Ok(EntityRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                name: row.get(2)?,
                attributes: row.get(3)?,
                first_seen: row.get(4)?,
                last_seen: row.get(5)?,
            })
        })?;

        let mut entities = Vec::new();
        for r in rows {
            entities.push(entity_from_row(r?));
        }
        Ok(entities)
    }
}

// ─── Stored event (flattened, with resolved subject_id) ─────────────

/// An event as stored in the database, with the subject as a resolved entity id
/// rather than an EntityRef.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    pub id: EventId,
    pub timestamp: Timestamp,
    pub source: CollectorSource,
    pub kind: EventKind,
    pub subject_id: EntityId,
    pub metadata: Attributes,
}

// ─── Internal row types for deserialization ──────────────────────────

struct EntityRow {
    id: String,
    kind: String,
    name: String,
    attributes: String,
    first_seen: i64,
    last_seen: i64,
}

struct EventRow {
    id: String,
    timestamp: i64,
    source: String,
    kind: String,
    subject_id: String,
    metadata: String,
}

struct EdgeRow {
    id: String,
    from_id: String,
    to_id: String,
    relation: String,
    strength: f64,
    created_at: i64,
    last_reinforced: i64,
}

fn parse_ulid(s: &str) -> ulid::Ulid {
    s.parse::<ulid::Ulid>().expect("invalid ULID in database")
}

fn entity_from_row(r: EntityRow) -> Entity {
    Entity {
        id: EntityId(parse_ulid(&r.id)),
        kind: serde_json::from_str(&r.kind).expect("invalid EntityKind in database"),
        name: r.name,
        attributes: serde_json::from_str(&r.attributes).unwrap_or_default(),
        first_seen: r.first_seen,
        last_seen: r.last_seen,
    }
}

fn stored_event_from_row(r: EventRow) -> StoredEvent {
    StoredEvent {
        id: EventId(parse_ulid(&r.id)),
        timestamp: r.timestamp,
        source: serde_json::from_str(&r.source).expect("invalid CollectorSource in database"),
        kind: serde_json::from_str(&r.kind).expect("invalid EventKind in database"),
        subject_id: EntityId(parse_ulid(&r.subject_id)),
        metadata: serde_json::from_str(&r.metadata).unwrap_or_default(),
    }
}

fn edge_from_row(r: EdgeRow) -> Edge {
    Edge {
        id: EdgeId(parse_ulid(&r.id)),
        from: EntityId(parse_ulid(&r.from_id)),
        to: EntityId(parse_ulid(&r.to_id)),
        relation: serde_json::from_str(&r.relation).expect("invalid Relation in database"),
        strength: r.strength as f32,
        created_at: r.created_at,
        last_reinforced: r.last_reinforced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cronos_model::{EntityRef, EventId};
    use std::collections::HashMap;

    fn make_entity(kind: EntityKind, name: &str, ts: Timestamp) -> Entity {
        Entity {
            id: EntityId::new(),
            kind,
            name: name.to_string(),
            attributes: HashMap::new(),
            first_seen: ts,
            last_seen: ts,
        }
    }

    fn make_edge(from: EntityId, to: EntityId, relation: Relation, ts: Timestamp) -> Edge {
        Edge {
            id: EdgeId::new(),
            from,
            to,
            relation,
            strength: 1.0,
            created_at: ts,
            last_reinforced: ts,
        }
    }

    #[test]
    fn insert_and_get_entity() {
        let repo = Repository::open_in_memory().unwrap();
        let entity = make_entity(EntityKind::File, "main.rs", 1000);

        repo.insert_entity(&entity).unwrap();

        let fetched = repo.get_entity(entity.id).unwrap().expect("entity not found");
        assert_eq!(fetched.id, entity.id);
        assert_eq!(fetched.kind, EntityKind::File);
        assert_eq!(fetched.name, "main.rs");
        assert_eq!(fetched.first_seen, 1000);
        assert_eq!(fetched.last_seen, 1000);
    }

    #[test]
    fn insert_entity_upserts_last_seen() {
        let repo = Repository::open_in_memory().unwrap();
        let mut entity = make_entity(EntityKind::File, "main.rs", 1000);
        repo.insert_entity(&entity).unwrap();

        // Update last_seen and name
        entity.last_seen = 2000;
        entity.name = "lib.rs".to_string();
        repo.insert_entity(&entity).unwrap();

        let fetched = repo.get_entity(entity.id).unwrap().expect("entity not found");
        assert_eq!(fetched.last_seen, 2000);
        assert_eq!(fetched.name, "lib.rs");
        // first_seen should remain unchanged because ON CONFLICT updates only last_seen, name, attributes
        assert_eq!(fetched.first_seen, 1000);
    }

    #[test]
    fn find_entity_by_kind_and_name() {
        let repo = Repository::open_in_memory().unwrap();
        let entity = make_entity(EntityKind::Project, "my-project", 1000);
        repo.insert_entity(&entity).unwrap();

        let found = repo
            .find_entity_by_kind_and_name(&EntityKind::Project, "my-project")
            .unwrap()
            .expect("entity not found");
        assert_eq!(found.id, entity.id);

        // Should not find with wrong kind
        let not_found = repo
            .find_entity_by_kind_and_name(&EntityKind::File, "my-project")
            .unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn insert_and_get_edge() {
        let repo = Repository::open_in_memory().unwrap();

        let e1 = make_entity(EntityKind::File, "a.rs", 1000);
        let e2 = make_entity(EntityKind::Project, "my-proj", 1000);
        repo.insert_entity(&e1).unwrap();
        repo.insert_entity(&e2).unwrap();

        let edge = make_edge(e1.id, e2.id, Relation::BelongsTo, 1000);
        repo.insert_edge(&edge).unwrap();

        let found = repo
            .find_edge(e1.id, e2.id, &Relation::BelongsTo)
            .unwrap()
            .expect("edge not found");
        assert_eq!(found.id, edge.id);
        assert_eq!(found.from, e1.id);
        assert_eq!(found.to, e2.id);
        assert_eq!(found.relation, Relation::BelongsTo);

        // edges_from
        let from_edges = repo.edges_from(e1.id).unwrap();
        assert_eq!(from_edges.len(), 1);

        // all_edges
        let all = repo.all_edges().unwrap();
        assert_eq!(all.len(), 1);

        // edge_count
        assert_eq!(repo.edge_count().unwrap(), 1);
    }

    #[test]
    fn insert_and_get_event() {
        let repo = Repository::open_in_memory().unwrap();

        let subject = make_entity(EntityKind::File, "main.rs", 1000);
        let ctx_entity = make_entity(EntityKind::Project, "proj", 1000);
        repo.insert_entity(&subject).unwrap();
        repo.insert_entity(&ctx_entity).unwrap();

        let event = Event {
            id: EventId::new(),
            timestamp: 1500,
            source: CollectorSource::Filesystem,
            kind: EventKind::FileModified,
            subject: EntityRef {
                kind: EntityKind::File,
                identity: "main.rs".to_string(),
                attributes: HashMap::new(),
            },
            context: vec![EntityRef {
                kind: EntityKind::Project,
                identity: "proj".to_string(),
                attributes: HashMap::new(),
            }],
            metadata: HashMap::new(),
        };

        repo.insert_event(&event, subject.id, &[ctx_entity.id])
            .unwrap();

        // event_count
        assert_eq!(repo.event_count().unwrap(), 1);

        // events_in_range
        let range = repo.events_in_range(1000, 2000).unwrap();
        assert_eq!(range.len(), 1);
        assert_eq!(range[0].id, event.id);
        assert_eq!(range[0].timestamp, 1500);
        assert_eq!(range[0].source, CollectorSource::Filesystem);
        assert_eq!(range[0].kind, EventKind::FileModified);
        assert_eq!(range[0].subject_id, subject.id);

        // recent_events
        let recent = repo.recent_events(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, event.id);
    }

    #[test]
    fn entity_count_works() {
        let repo = Repository::open_in_memory().unwrap();
        assert_eq!(repo.entity_count().unwrap(), 0);

        let e1 = make_entity(EntityKind::File, "a.rs", 1000);
        let e2 = make_entity(EntityKind::File, "b.rs", 1000);
        repo.insert_entity(&e1).unwrap();
        repo.insert_entity(&e2).unwrap();

        assert_eq!(repo.entity_count().unwrap(), 2);

        // all_entities
        let all = repo.all_entities().unwrap();
        assert_eq!(all.len(), 2);
    }
}
