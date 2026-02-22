use cronos_model::*;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub version: u8,
    pub id: String,
    pub kind: MessageKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageKind {
    // Collector -> Core
    EmitEvent { event: Event },
    CollectorHandshake {
        name: String,
        collector_version: String,
        source: CollectorSource,
    },
    Heartbeat,

    // CLI/UI -> Core
    Query { query: QueryRequest },
    Status,
    ListCollectors,
    SetTrackingPaused { paused: bool },

    // Core -> CLI/UI
    TrackingStatus { paused: bool },

    // Core -> Collector/CLI
    Ack { request_id: String },
    Error {
        request_id: String,
        code: ErrorCode,
        message: String,
    },
    QueryResult { response: QueryResponse },
    StatusResult { info: StatusInfo },
    CollectorList { collectors: Vec<CollectorInfo> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequest {
    pub kind: QueryKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueryKind {
    Search { text: String, limit: u32 },
    Timeline { from: Timestamp, to: Timestamp },
    Related { entity_id: EntityId, depth: u8 },
    Recent { limit: u32 },
    Sessions { from: Timestamp, to: Timestamp, limit: u32 },
    DaySummary { date: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub entities: Vec<Entity>,
    pub edges: Vec<Edge>,
    pub events: Vec<Event>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionInfo>,
}

/// A session as returned across the protocol boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub app_name: String,
    pub window_titles: Vec<String>,
    pub project: Option<String>,
    pub category: String,
    pub start_time: Timestamp,
    pub end_time: Timestamp,
    pub duration_secs: i64,
    pub event_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusInfo {
    pub uptime_secs: u64,
    pub entity_count: u64,
    pub edge_count: u64,
    pub event_count: u64,
    pub connected_collectors: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectorInfo {
    pub name: String,
    pub source: CollectorSource,
    pub connected: bool,
    pub last_heartbeat: Option<Timestamp>,
    pub events_sent: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidMessage,
    InternalError,
    NotFound,
    BadRequest,
}

impl Message {
    pub fn new(id: impl Into<String>, kind: MessageKind) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            id: id.into(),
            kind,
        }
    }

    pub fn ack(request_id: impl Into<String>) -> Self {
        let rid: String = request_id.into();
        Self::new(rid.clone(), MessageKind::Ack { request_id: rid })
    }

    pub fn error(
        request_id: impl Into<String>,
        code: ErrorCode,
        message: impl Into<String>,
    ) -> Self {
        let rid: String = request_id.into();
        Self::new(
            rid.clone(),
            MessageKind::Error {
                request_id: rid,
                code,
                message: message.into(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn message_serializes_emit_event() {
        let event = Event {
            id: EventId::new(),
            timestamp: 12345,
            source: CollectorSource::Filesystem,
            kind: EventKind::FileModified,
            subject: EntityRef {
                kind: EntityKind::File,
                identity: "/src/main.rs".to_string(),
                attributes: HashMap::new(),
            },
            context: vec![],
            metadata: HashMap::new(),
        };
        let msg = Message::new("r1", MessageKind::EmitEvent { event });
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, PROTOCOL_VERSION);
        match back.kind {
            MessageKind::EmitEvent { event } => {
                assert_eq!(event.kind, EventKind::FileModified);
            }
            _ => panic!("wrong kind"),
        }
    }

    #[test]
    fn message_serializes_query() {
        let msg = Message::new(
            "r2",
            MessageKind::Query {
                query: QueryRequest {
                    kind: QueryKind::Search {
                        text: "billing".to_string(),
                        limit: 10,
                    },
                },
            },
        );
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        match back.kind {
            MessageKind::Query { query } => match query.kind {
                QueryKind::Search { text, limit } => {
                    assert_eq!(text, "billing");
                    assert_eq!(limit, 10);
                }
                _ => panic!("wrong query kind"),
            },
            _ => panic!("wrong message kind"),
        }
    }
}
