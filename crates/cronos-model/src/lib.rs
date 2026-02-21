use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export ulid for consumers
pub use ulid::Ulid;

/// Millisecond-precision UTC timestamp
pub type Timestamp = i64;

/// Flexible attribute map
pub type Attributes = HashMap<String, serde_json::Value>;

// === IDs ===

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityId(pub Ulid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(pub Ulid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EdgeId(pub Ulid);

impl EntityId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl EventId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl EdgeId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for EntityId {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for EdgeId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for EventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for EdgeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// === Entity ===

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    pub kind: EntityKind,
    pub name: String,
    pub attributes: Attributes,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Project,
    File,
    Repository,
    Branch,
    Commit,
    Url,
    Domain,
    App,
    TerminalSession,
    TerminalCommand,
    Custom(String),
}

impl std::fmt::Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Project => write!(f, "project"),
            Self::File => write!(f, "file"),
            Self::Repository => write!(f, "repository"),
            Self::Branch => write!(f, "branch"),
            Self::Commit => write!(f, "commit"),
            Self::Url => write!(f, "url"),
            Self::Domain => write!(f, "domain"),
            Self::App => write!(f, "app"),
            Self::TerminalSession => write!(f, "terminal_session"),
            Self::TerminalCommand => write!(f, "terminal_command"),
            Self::Custom(s) => write!(f, "custom:{}", s),
        }
    }
}

// === Event ===

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: EventId,
    pub timestamp: Timestamp,
    pub source: CollectorSource,
    pub kind: EventKind,
    pub subject: EntityRef,
    pub context: Vec<EntityRef>,
    pub metadata: Attributes,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectorSource {
    Filesystem,
    Browser,
    Git,
    Terminal,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    FileOpened,
    FileModified,
    FileCreated,
    FileDeleted,
    UrlVisited,
    TabFocused,
    CommitCreated,
    BranchChanged,
    CommandExecuted,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityRef {
    pub kind: EntityKind,
    pub identity: String,
    pub attributes: Attributes,
}

// === Edge ===

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub from: EntityId,
    pub to: EntityId,
    pub relation: Relation,
    pub strength: f32,
    pub created_at: Timestamp,
    pub last_reinforced: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    BelongsTo,
    Contains,
    References,
    OccurredDuring,
    Visited,
    RelatedTo,
    Custom(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_id_is_unique() {
        let a = EntityId::new();
        let b = EntityId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn entity_serializes_to_json() {
        let entity = Entity {
            id: EntityId::new(),
            kind: EntityKind::File,
            name: "main.rs".to_string(),
            attributes: HashMap::from([("path".to_string(), serde_json::json!("/src/main.rs"))]),
            first_seen: 1000,
            last_seen: 2000,
        };
        let json = serde_json::to_string(&entity).unwrap();
        let back: Entity = serde_json::from_str(&json).unwrap();
        assert_eq!(entity, back);
    }

    #[test]
    fn event_serializes_roundtrip() {
        let event = Event {
            id: EventId::new(),
            timestamp: 12345,
            source: CollectorSource::Filesystem,
            kind: EventKind::FileModified,
            subject: EntityRef {
                kind: EntityKind::File,
                identity: "/home/user/projects/foo/main.rs".to_string(),
                attributes: HashMap::new(),
            },
            context: vec![EntityRef {
                kind: EntityKind::Project,
                identity: "/home/user/projects/foo".to_string(),
                attributes: HashMap::new(),
            }],
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }

    #[test]
    fn custom_entity_kind_serializes() {
        let kind = EntityKind::Custom("discord_channel".to_string());
        let json = serde_json::to_string(&kind).unwrap();
        let back: EntityKind = serde_json::from_str(&json).unwrap();
        assert_eq!(kind, back);
    }
}
