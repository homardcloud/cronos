use crate::graph::ContextGraph;
use crate::ingest::IngestPipeline;
use crate::linker::Linker;
use crate::storage::Repository;
use cronos_common::config::DaemonConfig;
use cronos_model::*;
use cronos_proto::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

pub struct Engine {
    repo: Mutex<Repository>,
    graph: Mutex<ContextGraph>,
    ingest: Mutex<IngestPipeline>,
    linker: Linker,
    start_time: Instant,
    collectors: Mutex<HashMap<String, CollectorInfo>>,
}

impl Engine {
    pub fn open(db_path: &Path, config: &DaemonConfig) -> anyhow::Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let repo = Repository::open(db_path)?;

        let entities = repo.all_entities()?;
        let edges = repo.all_edges()?;
        let graph = ContextGraph::rebuild(&entities, &edges);
        tracing::info!(
            entities = entities.len(),
            edges = edges.len(),
            "rebuilt in-memory graph"
        );

        Ok(Self {
            repo: Mutex::new(repo),
            graph: Mutex::new(graph),
            ingest: Mutex::new(IngestPipeline::new(config.dedup.window_ms)),
            linker: Linker::new(config.linker.temporal_window_ms),
            start_time: Instant::now(),
            collectors: Mutex::new(HashMap::new()),
        })
    }

    pub fn handle_message(&self, msg: Message) -> Message {
        let request_id = msg.id.clone();
        match msg.kind {
            MessageKind::EmitEvent { event } => self.handle_emit_event(request_id, event),
            MessageKind::CollectorHandshake {
                name,
                collector_version,
                source,
            } => self.handle_handshake(request_id, name, collector_version, source),
            MessageKind::Heartbeat => Message::ack(request_id),
            MessageKind::Query { query } => self.handle_query(request_id, query),
            MessageKind::Status => self.handle_status(request_id),
            MessageKind::ListCollectors => self.handle_list_collectors(request_id),
            _ => Message::error(request_id, ErrorCode::BadRequest, "unexpected message type"),
        }
    }

    fn handle_emit_event(&self, request_id: String, event: Event) -> Message {
        let processed = {
            let mut ingest = self.ingest.lock().unwrap();
            ingest.process(event)
        };
        let Some(event) = processed else {
            return Message::ack(request_id);
        };
        let result = {
            let repo = self.repo.lock().unwrap();
            let mut graph = self.graph.lock().unwrap();
            self.linker.link(&event, &repo, &mut graph)
        };
        match result {
            Ok(()) => {
                let source_str = format!("{:?}", event.source);
                if let Some(info) = self
                    .collectors
                    .lock()
                    .unwrap()
                    .values_mut()
                    .find(|c| format!("{:?}", c.source) == source_str)
                {
                    info.events_sent += 1;
                }
                Message::ack(request_id)
            }
            Err(e) => Message::error(request_id, ErrorCode::InternalError, e.to_string()),
        }
    }

    fn handle_handshake(
        &self,
        request_id: String,
        name: String,
        _version: String,
        source: CollectorSource,
    ) -> Message {
        let now = cronos_common::now_ms();
        self.collectors.lock().unwrap().insert(
            name.clone(),
            CollectorInfo {
                name,
                source,
                connected: true,
                last_heartbeat: Some(now),
                events_sent: 0,
            },
        );
        Message::ack(request_id)
    }

    fn handle_query(&self, request_id: String, query: QueryRequest) -> Message {
        let result = match query.kind {
            QueryKind::Search { text, limit } => {
                let repo = self.repo.lock().unwrap();
                repo.search_entities(&text, limit).map(|entities| {
                    QueryResponse {
                        entities,
                        edges: vec![],
                        events: vec![],
                    }
                })
            }
            QueryKind::Recent { limit } => {
                let repo = self.repo.lock().unwrap();
                repo.recent_events(limit).map(|stored| {
                    let events = stored
                        .into_iter()
                        .filter_map(|se| stored_event_to_event(&se, &repo))
                        .collect();
                    QueryResponse {
                        entities: vec![],
                        edges: vec![],
                        events,
                    }
                })
            }
            QueryKind::Timeline { from, to } => {
                let repo = self.repo.lock().unwrap();
                repo.events_in_range(from, to).map(|stored| {
                    let events = stored
                        .into_iter()
                        .filter_map(|se| stored_event_to_event(&se, &repo))
                        .collect();
                    QueryResponse {
                        entities: vec![],
                        edges: vec![],
                        events,
                    }
                })
            }
            QueryKind::Related { entity_id, depth } => {
                let graph = self.graph.lock().unwrap();
                let related_ids = graph.related(&entity_id, depth);
                let repo = self.repo.lock().unwrap();
                let mut entities = Vec::new();
                for id in related_ids {
                    if let Ok(Some(e)) = repo.get_entity(id) {
                        entities.push(e);
                    }
                }
                Ok(QueryResponse {
                    entities,
                    edges: vec![],
                    events: vec![],
                })
            }
        };
        match result {
            Ok(response) => Message::new(request_id, MessageKind::QueryResult { response }),
            Err(e) => Message::error(request_id, ErrorCode::InternalError, e.to_string()),
        }
    }

    fn handle_status(&self, request_id: String) -> Message {
        let repo = self.repo.lock().unwrap();
        let info = StatusInfo {
            uptime_secs: self.start_time.elapsed().as_secs(),
            entity_count: repo.entity_count().unwrap_or(0) as u64,
            edge_count: repo.edge_count().unwrap_or(0) as u64,
            event_count: repo.event_count().unwrap_or(0) as u64,
            connected_collectors: self.collectors.lock().unwrap().len() as u32,
        };
        Message::new(request_id, MessageKind::StatusResult { info })
    }

    fn handle_list_collectors(&self, request_id: String) -> Message {
        let collectors: Vec<CollectorInfo> =
            self.collectors.lock().unwrap().values().cloned().collect();
        Message::new(request_id, MessageKind::CollectorList { collectors })
    }
}

/// Convert a `StoredEvent` back into an `Event` by looking up the subject entity
/// from the repository to reconstruct the `EntityRef`.
fn stored_event_to_event(
    stored: &crate::storage::repo::StoredEvent,
    repo: &Repository,
) -> Option<Event> {
    let entity = repo.get_entity(stored.subject_id).ok()??;
    Some(Event {
        id: stored.id,
        timestamp: stored.timestamp,
        source: stored.source.clone(),
        kind: stored.kind.clone(),
        subject: EntityRef {
            kind: entity.kind,
            identity: entity.name,
            attributes: entity.attributes,
        },
        context: vec![],
        metadata: stored.metadata.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn make_engine(dir: &TempDir) -> Engine {
        let db_path = dir.path().join("test.db");
        let config = DaemonConfig::default();
        Engine::open(&db_path, &config).unwrap()
    }

    fn make_emit_event(identity: &str) -> Message {
        let event = Event {
            id: EventId::new(),
            timestamp: cronos_common::now_ms(),
            source: CollectorSource::Filesystem,
            kind: EventKind::FileModified,
            subject: EntityRef {
                kind: EntityKind::File,
                identity: identity.to_string(),
                attributes: HashMap::new(),
            },
            context: vec![EntityRef {
                kind: EntityKind::Project,
                identity: "test-project".to_string(),
                attributes: HashMap::new(),
            }],
            metadata: HashMap::new(),
        };
        Message::new("req-1", MessageKind::EmitEvent { event })
    }

    #[test]
    fn engine_opens_and_handles_status() {
        let dir = TempDir::new().unwrap();
        let engine = make_engine(&dir);
        let msg = Message::new("s1", MessageKind::Status);
        let resp = engine.handle_message(msg);
        match resp.kind {
            MessageKind::StatusResult { info } => {
                assert_eq!(info.entity_count, 0);
                assert_eq!(info.edge_count, 0);
                assert_eq!(info.event_count, 0);
                assert_eq!(info.connected_collectors, 0);
            }
            other => panic!("expected StatusResult, got {:?}", other),
        }
    }

    #[test]
    fn engine_handles_heartbeat() {
        let dir = TempDir::new().unwrap();
        let engine = make_engine(&dir);
        let msg = Message::new("h1", MessageKind::Heartbeat);
        let resp = engine.handle_message(msg);
        match resp.kind {
            MessageKind::Ack { request_id } => assert_eq!(request_id, "h1"),
            other => panic!("expected Ack, got {:?}", other),
        }
    }

    #[test]
    fn engine_handles_collector_handshake() {
        let dir = TempDir::new().unwrap();
        let engine = make_engine(&dir);
        let msg = Message::new(
            "c1",
            MessageKind::CollectorHandshake {
                name: "fs-collector".to_string(),
                collector_version: "0.1.0".to_string(),
                source: CollectorSource::Filesystem,
            },
        );
        let resp = engine.handle_message(msg);
        match &resp.kind {
            MessageKind::Ack { request_id } => assert_eq!(request_id, "c1"),
            other => panic!("expected Ack, got {:?}", other),
        }

        // Verify collector is registered
        let list_msg = Message::new("c2", MessageKind::ListCollectors);
        let list_resp = engine.handle_message(list_msg);
        match list_resp.kind {
            MessageKind::CollectorList { collectors } => {
                assert_eq!(collectors.len(), 1);
                assert_eq!(collectors[0].name, "fs-collector");
                assert!(collectors[0].connected);
            }
            other => panic!("expected CollectorList, got {:?}", other),
        }
    }

    #[test]
    fn engine_handles_emit_event() {
        let dir = TempDir::new().unwrap();
        let engine = make_engine(&dir);
        let msg = make_emit_event("/src/main.rs");
        let resp = engine.handle_message(msg);
        match &resp.kind {
            MessageKind::Ack { .. } => {}
            other => panic!("expected Ack, got {:?}", other),
        }

        // Verify status shows the entities and event
        let status_msg = Message::new("s1", MessageKind::Status);
        let status_resp = engine.handle_message(status_msg);
        match status_resp.kind {
            MessageKind::StatusResult { info } => {
                assert_eq!(info.entity_count, 2); // file + project
                assert_eq!(info.event_count, 1);
                assert_eq!(info.edge_count, 1);
            }
            other => panic!("expected StatusResult, got {:?}", other),
        }
    }

    #[test]
    fn engine_handles_search_query() {
        let dir = TempDir::new().unwrap();
        let engine = make_engine(&dir);

        // Emit an event to create some entities
        let msg = make_emit_event("/src/main.rs");
        engine.handle_message(msg);

        // Search for the entity
        let query_msg = Message::new(
            "q1",
            MessageKind::Query {
                query: QueryRequest {
                    kind: QueryKind::Search {
                        text: "main".to_string(),
                        limit: 10,
                    },
                },
            },
        );
        let resp = engine.handle_message(query_msg);
        match resp.kind {
            MessageKind::QueryResult { response } => {
                assert!(!response.entities.is_empty());
            }
            other => panic!("expected QueryResult, got {:?}", other),
        }
    }

    #[test]
    fn engine_handles_recent_query() {
        let dir = TempDir::new().unwrap();
        let engine = make_engine(&dir);

        let msg = make_emit_event("/src/lib.rs");
        engine.handle_message(msg);

        let query_msg = Message::new(
            "q2",
            MessageKind::Query {
                query: QueryRequest {
                    kind: QueryKind::Recent { limit: 10 },
                },
            },
        );
        let resp = engine.handle_message(query_msg);
        match resp.kind {
            MessageKind::QueryResult { response } => {
                assert_eq!(response.events.len(), 1);
            }
            other => panic!("expected QueryResult, got {:?}", other),
        }
    }

    #[test]
    fn engine_handles_unexpected_message() {
        let dir = TempDir::new().unwrap();
        let engine = make_engine(&dir);
        let msg = Message::new("x1", MessageKind::Ack { request_id: "x".to_string() });
        let resp = engine.handle_message(msg);
        match resp.kind {
            MessageKind::Error { code, .. } => {
                assert!(matches!(code, ErrorCode::BadRequest));
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }
}
