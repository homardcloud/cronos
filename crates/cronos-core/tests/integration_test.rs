use cronos_common::config::DaemonConfig;
use cronos_core::engine::Engine;
use cronos_model::*;
use cronos_proto::*;
use std::collections::HashMap;
use tempfile::TempDir;

/// Helper: create an Engine backed by a temp directory.
fn make_engine(dir: &TempDir) -> Engine {
    let db_path = dir.path().join("test.db");
    let config = DaemonConfig::default();
    Engine::open(&db_path, &config).unwrap()
}

/// Helper: build an EmitEvent message with a File subject and a Project context entity.
fn make_emit_event(request_id: &str, identity: &str, timestamp: Timestamp) -> Message {
    let event = Event {
        id: EventId::new(),
        timestamp,
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
    Message::new(request_id, MessageKind::EmitEvent { event })
}

/// End-to-end: emit an event, then verify status counts and recent query.
#[test]
fn engine_handles_event_and_query() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);
    let ts = cronos_common::now_ms();

    // 1. Emit an event (File subject + Project context)
    let emit_msg = make_emit_event("e1", "/src/main.rs", ts);
    let emit_resp = engine.handle_message(emit_msg);
    match &emit_resp.kind {
        MessageKind::Ack { request_id } => assert_eq!(request_id, "e1"),
        other => panic!("expected Ack for EmitEvent, got {:?}", other),
    }

    // 2. Status should show 2 entities (File + Project), 1 edge, 1 event
    let status_msg = Message::new("s1", MessageKind::Status);
    let status_resp = engine.handle_message(status_msg);
    match status_resp.kind {
        MessageKind::StatusResult { info } => {
            assert_eq!(info.entity_count, 2, "expected 2 entities (file + project)");
            assert_eq!(info.edge_count, 1, "expected 1 edge between file and project");
            assert_eq!(info.event_count, 1, "expected 1 event stored");
        }
        other => panic!("expected StatusResult, got {:?}", other),
    }

    // 3. Recent query should return 1 event
    let query_msg = Message::new(
        "q1",
        MessageKind::Query {
            query: QueryRequest {
                kind: QueryKind::Recent { limit: 10 },
            },
        },
    );
    let query_resp = engine.handle_message(query_msg);
    match query_resp.kind {
        MessageKind::QueryResult { response } => {
            assert_eq!(response.events.len(), 1, "expected 1 recent event");
        }
        other => panic!("expected QueryResult, got {:?}", other),
    }
}

/// End-to-end: two events with same source + subject within the dedup window
/// should result in only 1 stored event.
#[test]
fn engine_deduplicates_rapid_events() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);
    let ts = cronos_common::now_ms();

    // Emit first event
    let msg1 = make_emit_event("d1", "/src/main.rs", ts);
    let resp1 = engine.handle_message(msg1);
    match &resp1.kind {
        MessageKind::Ack { .. } => {}
        other => panic!("expected Ack for first EmitEvent, got {:?}", other),
    }

    // Emit second event with same source + subject, within the 1000ms dedup window
    let msg2 = make_emit_event("d2", "/src/main.rs", ts + 500);
    let resp2 = engine.handle_message(msg2);
    match &resp2.kind {
        MessageKind::Ack { .. } => {}
        other => panic!("expected Ack for second EmitEvent, got {:?}", other),
    }

    // Status should show only 1 event (the second was deduped)
    let status_msg = Message::new("s1", MessageKind::Status);
    let status_resp = engine.handle_message(status_msg);
    match status_resp.kind {
        MessageKind::StatusResult { info } => {
            assert_eq!(
                info.event_count, 1,
                "expected 1 event (second should be deduped)"
            );
        }
        other => panic!("expected StatusResult, got {:?}", other),
    }
}

/// End-to-end: send a CollectorHandshake and verify it appears in ListCollectors.
#[test]
fn engine_handles_collector_handshake() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    // 1. Send CollectorHandshake
    let handshake_msg = Message::new(
        "h1",
        MessageKind::CollectorHandshake {
            name: "fs-collector".to_string(),
            collector_version: "0.1.0".to_string(),
            source: CollectorSource::Filesystem,
        },
    );
    let handshake_resp = engine.handle_message(handshake_msg);
    match &handshake_resp.kind {
        MessageKind::Ack { request_id } => assert_eq!(request_id, "h1"),
        other => panic!("expected Ack for CollectorHandshake, got {:?}", other),
    }

    // 2. ListCollectors should return 1 collector with correct name
    let list_msg = Message::new("l1", MessageKind::ListCollectors);
    let list_resp = engine.handle_message(list_msg);
    match list_resp.kind {
        MessageKind::CollectorList { collectors } => {
            assert_eq!(collectors.len(), 1, "expected 1 registered collector");
            assert_eq!(collectors[0].name, "fs-collector");
            assert!(collectors[0].connected);
        }
        other => panic!("expected CollectorList, got {:?}", other),
    }
}
