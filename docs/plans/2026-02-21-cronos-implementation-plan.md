# Cronos Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the Cronos local-first context engine — a daemon that observes user activity via opt-in collectors and builds a temporal context graph.

**Architecture:** Plugin-based: a core daemon (cronos-core) handles graph storage, event ingestion, and queries via Unix socket. Separate collector binaries connect to the daemon to emit events. A single `cronos` CLI binary provides both daemon and query subcommands.

**Tech Stack:** Rust, tokio, rusqlite, petgraph, serde/serde_json, clap, notify (inotify), TOML config, tracing

---

## Task 1: Cargo Workspace + cronos-model

Set up the monorepo workspace and implement all domain types.

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/cronos-model/Cargo.toml`
- Create: `crates/cronos-model/src/lib.rs`

**Step 1: Create workspace root Cargo.toml**

```toml
[workspace]
resolver = "2"
members = [
    "crates/cronos-model",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
anyhow = "1.0"
chrono = { version = "0.4", features = ["serde"] }
ulid = { version = "1.2", features = ["serde"] }
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4.5", features = ["derive"] }
toml = "1.0"
rusqlite = { version = "0.38", features = ["bundled"] }
petgraph = "0.8"
notify = "8.2"
directories = "6.0"
```

**Step 2: Create cronos-model crate**

```toml
# crates/cronos-model/Cargo.toml
[package]
name = "cronos-model"
version.workspace = true
edition.workspace = true

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
ulid = { workspace = true }
chrono = { workspace = true }
```

**Step 3: Implement domain types in `crates/cronos-model/src/lib.rs`**

```rust
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
            Self::Custom(s) => write!(f, "custom:{}", s),
            other => {
                let s = serde_json::to_value(other)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| format!("{:?}", other));
                write!(f, "{}", s)
            }
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
```

**Step 4: Write tests for cronos-model**

Add to the bottom of `crates/cronos-model/src/lib.rs`:

```rust
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
    fn edge_strength_bounds() {
        let edge = Edge {
            id: EdgeId::new(),
            from: EntityId::new(),
            to: EntityId::new(),
            relation: Relation::BelongsTo,
            strength: 0.5,
            created_at: 1000,
            last_reinforced: 2000,
        };
        assert!(edge.strength >= 0.0 && edge.strength <= 1.0);
    }

    #[test]
    fn custom_entity_kind_serializes() {
        let kind = EntityKind::Custom("discord_channel".to_string());
        let json = serde_json::to_string(&kind).unwrap();
        let back: EntityKind = serde_json::from_str(&json).unwrap();
        assert_eq!(kind, back);
    }
}
```

**Step 5: Run tests**

Run: `cargo test -p cronos-model`
Expected: All 5 tests pass.

**Step 6: Commit**

```bash
git add Cargo.toml crates/cronos-model/
git commit -m "feat: add workspace and cronos-model with domain types"
```

---

## Task 2: cronos-common (Shared Utilities)

Config loading, XDG paths, error types, ID generation, tracing setup.

**Files:**
- Create: `crates/cronos-common/Cargo.toml`
- Create: `crates/cronos-common/src/lib.rs`
- Create: `crates/cronos-common/src/config.rs`
- Create: `crates/cronos-common/src/paths.rs`
- Create: `crates/cronos-common/src/error.rs`
- Create: `config/cronos.default.toml`
- Modify: `Cargo.toml` (add to workspace members)

**Step 1: Add cronos-common to workspace members**

Add `"crates/cronos-common"` to the `members` array in the root `Cargo.toml`.

**Step 2: Create cronos-common/Cargo.toml**

```toml
[package]
name = "cronos-common"
version.workspace = true
edition.workspace = true

[dependencies]
cronos-model = { path = "../cronos-model" }
serde = { workspace = true }
serde_json = { workspace = true }
toml = { workspace = true }
thiserror = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
directories = { workspace = true }

[dev-dependencies]
tempfile = "3"
```

**Step 3: Implement `crates/cronos-common/src/error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CronosError {
    #[error("config error: {0}")]
    Config(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("connection error: {0}")]
    Connection(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CronosError>;
```

**Step 4: Implement `crates/cronos-common/src/paths.rs`**

```rust
use directories::ProjectDirs;
use std::path::PathBuf;

const QUALIFIER: &str = "";
const ORG: &str = "";
const APP: &str = "cronos";

/// Resolved XDG-compliant paths for Cronos runtime
#[derive(Debug, Clone)]
pub struct CronosPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
    pub db_file: PathBuf,
    pub runtime_dir: PathBuf,
    pub socket_file: PathBuf,
}

impl CronosPaths {
    pub fn resolve() -> crate::error::Result<Self> {
        let dirs = ProjectDirs::from(QUALIFIER, ORG, APP)
            .ok_or_else(|| crate::error::CronosError::Config(
                "cannot determine XDG directories".to_string(),
            ))?;

        let config_dir = dirs.config_dir().to_path_buf();
        let data_dir = dirs.data_dir().to_path_buf();

        // XDG_RUNTIME_DIR or fallback to /tmp/cronos-<uid>
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .map(|d| PathBuf::from(d).join("cronos"))
            .unwrap_or_else(|_| {
                let uid = unsafe { libc::getuid() };
                PathBuf::from(format!("/tmp/cronos-{}", uid))
            });

        Ok(Self {
            config_file: config_dir.join("config.toml"),
            config_dir,
            db_file: data_dir.join("cronos.db"),
            data_dir,
            socket_file: runtime_dir.join("cronos.sock"),
            runtime_dir,
        })
    }
}
```

Add `libc = "0.2"` to cronos-common dependencies in Cargo.toml. Add `libc = "0.2"` to `[workspace.dependencies]` in root Cargo.toml.

**Step 5: Implement `crates/cronos-common/src/config.rs`**

```rust
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronosConfig {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub collectors: CollectorsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default)]
    pub socket_path: String,
    #[serde(default)]
    pub db_path: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_channel_size")]
    pub event_channel_size: usize,
    #[serde(default)]
    pub dedup: DedupConfig,
    #[serde(default)]
    pub linker: LinkerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupConfig {
    #[serde(default = "default_dedup_window")]
    pub window_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkerConfig {
    #[serde(default = "default_temporal_window")]
    pub temporal_window_ms: u64,
    #[serde(default = "default_min_edge_strength")]
    pub min_edge_strength: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectorsConfig {
    #[serde(default)]
    pub fs: FsCollectorConfig,
    #[serde(default)]
    pub browser: BrowserCollectorConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsCollectorConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_watch_paths")]
    pub watch_paths: Vec<String>,
    #[serde(default = "default_ignore_patterns")]
    pub ignore_patterns: Vec<String>,
    #[serde(default = "default_debounce")]
    pub debounce_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCollectorConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_browser_port")]
    pub listen_port: u16,
    #[serde(default)]
    pub ignore_domains: Vec<String>,
    #[serde(default = "default_dwell_time")]
    pub min_dwell_time_ms: u64,
}

// Default functions
fn default_log_level() -> String { "info".to_string() }
fn default_channel_size() -> usize { 4096 }
fn default_dedup_window() -> u64 { 1000 }
fn default_temporal_window() -> u64 { 300_000 }
fn default_min_edge_strength() -> f32 { 0.1 }
fn default_true() -> bool { true }
fn default_watch_paths() -> Vec<String> { vec!["~/projects".to_string()] }
fn default_ignore_patterns() -> Vec<String> {
    vec![
        "*/node_modules/*".to_string(),
        "*/.git/objects/*".to_string(),
        "*/target/*".to_string(),
        "*/.cache/*".to_string(),
        "*.swp".to_string(),
        "*.tmp".to_string(),
    ]
}
fn default_debounce() -> u64 { 500 }
fn default_browser_port() -> u16 { 19280 }
fn default_dwell_time() -> u64 { 3000 }

// Default impls
impl Default for CronosConfig {
    fn default() -> Self {
        Self {
            daemon: DaemonConfig::default(),
            collectors: CollectorsConfig::default(),
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: String::new(),
            db_path: String::new(),
            log_level: default_log_level(),
            event_channel_size: default_channel_size(),
            dedup: DedupConfig::default(),
            linker: LinkerConfig::default(),
        }
    }
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self { window_ms: default_dedup_window() }
    }
}

impl Default for LinkerConfig {
    fn default() -> Self {
        Self {
            temporal_window_ms: default_temporal_window(),
            min_edge_strength: default_min_edge_strength(),
        }
    }
}

impl Default for CollectorsConfig {
    fn default() -> Self {
        Self {
            fs: FsCollectorConfig::default(),
            browser: BrowserCollectorConfig::default(),
        }
    }
}

impl Default for FsCollectorConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            watch_paths: default_watch_paths(),
            ignore_patterns: default_ignore_patterns(),
            debounce_ms: default_debounce(),
        }
    }
}

impl Default for BrowserCollectorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen_port: default_browser_port(),
            ignore_domains: Vec::new(),
            min_dwell_time_ms: default_dwell_time(),
        }
    }
}

impl CronosConfig {
    /// Load config: defaults → file → env overrides
    pub fn load(path: &Path) -> crate::error::Result<Self> {
        let mut config = Self::default();

        if path.exists() {
            let content = std::fs::read_to_string(path)
                .map_err(|e| crate::error::CronosError::Config(format!("read config: {}", e)))?;
            config = toml::from_str(&content)
                .map_err(|e| crate::error::CronosError::Config(format!("parse config: {}", e)))?;
        }

        // Env overrides
        if let Ok(v) = std::env::var("CRONOS_LOG_LEVEL") {
            config.daemon.log_level = v;
        }
        if let Ok(v) = std::env::var("CRONOS_SOCKET_PATH") {
            config.daemon.socket_path = v;
        }
        if let Ok(v) = std::env::var("CRONOS_DB_PATH") {
            config.daemon.db_path = v;
        }

        Ok(config)
    }
}
```

**Step 6: Implement `crates/cronos-common/src/lib.rs`**

```rust
pub mod config;
pub mod error;
pub mod paths;

pub use config::CronosConfig;
pub use error::{CronosError, Result};
pub use paths::CronosPaths;

/// Initialize tracing with the given log level filter
pub fn init_tracing(level: &str) {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();
}

/// Get current time as millisecond timestamp
pub fn now_ms() -> cronos_model::Timestamp {
    chrono::Utc::now().timestamp_millis()
}
```

Add `chrono = { workspace = true }` to cronos-common dependencies.

**Step 7: Create `config/cronos.default.toml`**

```toml
[daemon]
# socket_path = ""                  # default: $XDG_RUNTIME_DIR/cronos/cronos.sock
# db_path = ""                      # default: $XDG_DATA_HOME/cronos/cronos.db
log_level = "info"
event_channel_size = 4096

[daemon.dedup]
window_ms = 1000

[daemon.linker]
temporal_window_ms = 300000         # 5 min
min_edge_strength = 0.1

[collectors.fs]
enabled = true
watch_paths = ["~/projects"]
ignore_patterns = [
    "*/node_modules/*",
    "*/.git/objects/*",
    "*/target/*",
    "*/.cache/*",
    "*.swp",
    "*.tmp",
]
debounce_ms = 500

[collectors.browser]
enabled = false
listen_port = 19280
ignore_domains = []
min_dwell_time_ms = 3000
```

**Step 8: Write tests for cronos-common**

Add tests at the bottom of each module:

In `config.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn default_config_is_valid() {
        let config = CronosConfig::default();
        assert_eq!(config.daemon.log_level, "info");
        assert_eq!(config.daemon.event_channel_size, 4096);
        assert!(config.collectors.fs.enabled);
        assert!(!config.collectors.browser.enabled);
    }

    #[test]
    fn config_loads_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "[daemon]\nlog_level = \"debug\"").unwrap();
        let config = CronosConfig::load(&path).unwrap();
        assert_eq!(config.daemon.log_level, "debug");
    }

    #[test]
    fn config_defaults_when_file_missing() {
        let path = std::path::Path::new("/tmp/nonexistent-cronos-test.toml");
        let config = CronosConfig::load(path).unwrap();
        assert_eq!(config.daemon.log_level, "info");
    }

    #[test]
    fn config_serializes_to_toml() {
        let config = CronosConfig::default();
        let s = toml::to_string_pretty(&config).unwrap();
        assert!(s.contains("log_level"));
    }
}
```

**Step 9: Run tests**

Run: `cargo test -p cronos-common`
Expected: All tests pass.

**Step 10: Commit**

```bash
git add Cargo.toml crates/cronos-common/ config/
git commit -m "feat: add cronos-common with config, XDG paths, and error types"
```

---

## Task 3: cronos-proto (IPC Protocol)

Protocol messages, length-prefixed framing, serialization.

**Files:**
- Create: `crates/cronos-proto/Cargo.toml`
- Create: `crates/cronos-proto/src/lib.rs`
- Create: `crates/cronos-proto/src/message.rs`
- Create: `crates/cronos-proto/src/frame.rs`
- Modify: `Cargo.toml` (add to workspace members)

**Step 1: Add to workspace members and create Cargo.toml**

```toml
# crates/cronos-proto/Cargo.toml
[package]
name = "cronos-proto"
version.workspace = true
edition.workspace = true

[dependencies]
cronos-model = { path = "../cronos-model" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
```

**Step 2: Implement `crates/cronos-proto/src/message.rs`**

```rust
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

    // CLI -> Core
    Query { query: QueryRequest },
    Status,
    ListCollectors,

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub entities: Vec<Entity>,
    pub edges: Vec<Edge>,
    pub events: Vec<Event>,
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
        Self::new(
            rid.clone(),
            MessageKind::Ack { request_id: rid },
        )
    }

    pub fn error(request_id: impl Into<String>, code: ErrorCode, message: impl Into<String>) -> Self {
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
```

**Step 3: Implement `crates/cronos-proto/src/frame.rs`**

```rust
use crate::message::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024; // 16 MB

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame too large: {size} bytes (max {MAX_FRAME_SIZE})")]
    TooLarge { size: u32 },
    #[error("connection closed")]
    ConnectionClosed,
}

/// Write a length-prefixed JSON message
pub async fn write_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    msg: &Message,
) -> Result<(), FrameError> {
    let payload = serde_json::to_vec(msg)?;
    let len = payload.len() as u32;
    if len > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge { size: len });
    }
    writer.write_all(&len.to_le_bytes()).await?;
    writer.write_all(&payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a length-prefixed JSON message
pub async fn read_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<Message, FrameError> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(FrameError::ConnectionClosed);
        }
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge { size: len });
    }
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload).await?;
    let msg = serde_json::from_slice(&payload)?;
    Ok(msg)
}
```

**Step 4: Implement `crates/cronos-proto/src/lib.rs`**

```rust
pub mod frame;
pub mod message;

pub use frame::{read_frame, write_frame, FrameError};
pub use message::*;
```

**Step 5: Write tests for cronos-proto**

Add to `frame.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, MessageKind, PROTOCOL_VERSION};

    #[tokio::test]
    async fn frame_roundtrip() {
        let msg = Message::new("req-1".to_string(), MessageKind::Status);

        let mut buf = Vec::new();
        write_frame(&mut buf, &msg).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded = read_frame(&mut cursor).await.unwrap();

        assert_eq!(decoded.version, PROTOCOL_VERSION);
        assert_eq!(decoded.id, "req-1");
    }

    #[tokio::test]
    async fn frame_ack_roundtrip() {
        let msg = Message::ack("req-42");

        let mut buf = Vec::new();
        write_frame(&mut buf, &msg).await.unwrap();

        let mut cursor = std::io::Cursor::new(buf);
        let decoded = read_frame(&mut cursor).await.unwrap();

        match decoded.kind {
            MessageKind::Ack { request_id } => assert_eq!(request_id, "req-42"),
            other => panic!("expected Ack, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn frame_connection_closed() {
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        let result = read_frame(&mut cursor).await;
        assert!(matches!(result, Err(FrameError::ConnectionClosed)));
    }
}
```

Add to `message.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use cronos_model::*;
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
        let msg = Message::new("r2", MessageKind::Query {
            query: QueryRequest {
                kind: QueryKind::Search {
                    text: "billing".to_string(),
                    limit: 10,
                },
            },
        });
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
```

**Step 6: Run tests**

Run: `cargo test -p cronos-proto`
Expected: All tests pass.

**Step 7: Commit**

```bash
git add Cargo.toml crates/cronos-proto/
git commit -m "feat: add cronos-proto with IPC messages and length-prefixed framing"
```

---

## Task 4: cronos-core/storage (SQLite Layer)

SQLite schema, migrations, CRUD for entities/events/edges.

**Files:**
- Create: `crates/cronos-core/Cargo.toml`
- Create: `crates/cronos-core/src/lib.rs`
- Create: `crates/cronos-core/src/storage/mod.rs`
- Create: `crates/cronos-core/src/storage/migrations.rs`
- Create: `crates/cronos-core/src/storage/repo.rs`
- Modify: `Cargo.toml` (add to workspace members)

**Step 1: Add to workspace and create Cargo.toml**

```toml
# crates/cronos-core/Cargo.toml
[package]
name = "cronos-core"
version.workspace = true
edition.workspace = true

[dependencies]
cronos-model = { path = "../cronos-model" }
cronos-proto = { path = "../cronos-proto" }
cronos-common = { path = "../cronos-common" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
anyhow = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
rusqlite = { workspace = true }
petgraph = { workspace = true }
chrono = { workspace = true }
ulid = { workspace = true }

[dev-dependencies]
tempfile = "3"
```

**Step 2: Implement `crates/cronos-core/src/storage/migrations.rs`**

```rust
use rusqlite::Connection;

const CURRENT_VERSION: i32 = 1;

pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)",
        [],
    )?;

    let version: i32 = conn
        .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_version", [], |row| {
            row.get(0)
        })?;

    if version < 1 {
        migrate_v1(conn)?;
    }

    Ok(())
}

fn migrate_v1(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE entities (
            id          TEXT PRIMARY KEY,
            kind        TEXT NOT NULL,
            name        TEXT NOT NULL,
            attributes  TEXT NOT NULL DEFAULT '{}',
            first_seen  INTEGER NOT NULL,
            last_seen   INTEGER NOT NULL
        );
        CREATE INDEX idx_entities_kind ON entities(kind);
        CREATE INDEX idx_entities_name ON entities(name);
        CREATE INDEX idx_entities_last_seen ON entities(last_seen);

        CREATE TABLE events (
            id          TEXT PRIMARY KEY,
            timestamp   INTEGER NOT NULL,
            source      TEXT NOT NULL,
            kind        TEXT NOT NULL,
            subject_id  TEXT NOT NULL REFERENCES entities(id),
            metadata    TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX idx_events_timestamp ON events(timestamp);
        CREATE INDEX idx_events_source ON events(source);
        CREATE INDEX idx_events_subject ON events(subject_id);

        CREATE TABLE event_context (
            event_id    TEXT NOT NULL REFERENCES events(id),
            entity_id   TEXT NOT NULL REFERENCES entities(id),
            PRIMARY KEY (event_id, entity_id)
        );

        CREATE TABLE edges (
            id              TEXT PRIMARY KEY,
            from_id         TEXT NOT NULL REFERENCES entities(id),
            to_id           TEXT NOT NULL REFERENCES entities(id),
            relation        TEXT NOT NULL,
            strength        REAL NOT NULL DEFAULT 0.5,
            created_at      INTEGER NOT NULL,
            last_reinforced INTEGER NOT NULL
        );
        CREATE INDEX idx_edges_from ON edges(from_id);
        CREATE INDEX idx_edges_to ON edges(to_id);
        CREATE INDEX idx_edges_relation ON edges(relation);

        CREATE VIRTUAL TABLE entities_fts USING fts5(
            name,
            attributes,
            content='entities',
            content_rowid='rowid'
        );

        INSERT INTO schema_version (version) VALUES (1);
        ",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_run_on_fresh_db() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let version: i32 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap(); // should not error
        let version: i32 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);
    }
}
```

**Step 3: Implement `crates/cronos-core/src/storage/repo.rs`**

```rust
use cronos_model::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

pub struct Repository {
    conn: Connection,
}

impl Repository {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    // === Entities ===

    pub fn insert_entity(&self, entity: &Entity) -> rusqlite::Result<()> {
        let kind_str = serde_json::to_value(&entity.kind)
            .unwrap()
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let attrs = serde_json::to_string(&entity.attributes).unwrap();

        self.conn.execute(
            "INSERT INTO entities (id, kind, name, attributes, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 attributes = excluded.attributes,
                 last_seen = excluded.last_seen",
            params![
                entity.id.to_string(),
                kind_str,
                entity.name,
                attrs,
                entity.first_seen,
                entity.last_seen,
            ],
        )?;

        // Update FTS index
        self.conn.execute(
            "INSERT INTO entities_fts(rowid, name, attributes)
             SELECT rowid, name, attributes FROM entities WHERE id = ?1
             ON CONFLICT DO NOTHING",
            params![entity.id.to_string()],
        )?;

        Ok(())
    }

    pub fn get_entity(&self, id: &EntityId) -> rusqlite::Result<Option<Entity>> {
        self.conn
            .query_row(
                "SELECT id, kind, name, attributes, first_seen, last_seen FROM entities WHERE id = ?1",
                params![id.to_string()],
                |row| {
                    Ok(Entity {
                        id: EntityId(row.get::<_, String>(0)?.parse().unwrap()),
                        kind: serde_json::from_value(serde_json::Value::String(row.get(1)?)).unwrap(),
                        name: row.get(2)?,
                        attributes: serde_json::from_str(&row.get::<_, String>(3)?).unwrap(),
                        first_seen: row.get(4)?,
                        last_seen: row.get(5)?,
                    })
                },
            )
            .optional()
    }

    pub fn find_entity_by_kind_and_name(&self, kind: &EntityKind, name: &str) -> rusqlite::Result<Option<Entity>> {
        let kind_str = serde_json::to_value(kind)
            .unwrap()
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        self.conn
            .query_row(
                "SELECT id, kind, name, attributes, first_seen, last_seen FROM entities WHERE kind = ?1 AND name = ?2",
                params![kind_str, name],
                |row| {
                    Ok(Entity {
                        id: EntityId(row.get::<_, String>(0)?.parse().unwrap()),
                        kind: serde_json::from_value(serde_json::Value::String(row.get(1)?)).unwrap(),
                        name: row.get(2)?,
                        attributes: serde_json::from_str(&row.get::<_, String>(3)?).unwrap(),
                        first_seen: row.get(4)?,
                        last_seen: row.get(5)?,
                    })
                },
            )
            .optional()
    }

    pub fn all_entities(&self) -> rusqlite::Result<Vec<Entity>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, name, attributes, first_seen, last_seen FROM entities",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Entity {
                id: EntityId(row.get::<_, String>(0)?.parse().unwrap()),
                kind: serde_json::from_value(serde_json::Value::String(row.get(1)?)).unwrap(),
                name: row.get(2)?,
                attributes: serde_json::from_str(&row.get::<_, String>(3)?).unwrap(),
                first_seen: row.get(4)?,
                last_seen: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn entity_count(&self) -> rusqlite::Result<u64> {
        self.conn.query_row("SELECT COUNT(*) FROM entities", [], |r| r.get(0))
    }

    // === Events ===

    pub fn insert_event(&self, event: &Event, subject_entity_id: &EntityId, context_entity_ids: &[EntityId]) -> rusqlite::Result<()> {
        let source_str = serde_json::to_value(&event.source)
            .unwrap()
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let kind_str = serde_json::to_value(&event.kind)
            .unwrap()
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let metadata = serde_json::to_string(&event.metadata).unwrap();

        self.conn.execute(
            "INSERT INTO events (id, timestamp, source, kind, subject_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.id.to_string(),
                event.timestamp,
                source_str,
                kind_str,
                subject_entity_id.to_string(),
                metadata,
            ],
        )?;

        for ctx_id in context_entity_ids {
            self.conn.execute(
                "INSERT OR IGNORE INTO event_context (event_id, entity_id) VALUES (?1, ?2)",
                params![event.id.to_string(), ctx_id.to_string()],
            )?;
        }

        Ok(())
    }

    pub fn events_in_range(&self, from: Timestamp, to: Timestamp) -> rusqlite::Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, source, kind, subject_id, metadata FROM events
             WHERE timestamp >= ?1 AND timestamp <= ?2 ORDER BY timestamp",
        )?;
        let rows = stmt.query_map(params![from, to], |row| {
            let subject_id_str: String = row.get(4)?;
            Ok(Event {
                id: EventId(row.get::<_, String>(0)?.parse().unwrap()),
                timestamp: row.get(1)?,
                source: serde_json::from_value(serde_json::Value::String(row.get(2)?)).unwrap(),
                kind: serde_json::from_value(serde_json::Value::String(row.get(3)?)).unwrap(),
                subject: EntityRef {
                    kind: EntityKind::File, // placeholder — subject is resolved via entity table
                    identity: subject_id_str,
                    attributes: HashMap::new(),
                },
                context: vec![],
                metadata: serde_json::from_str(&row.get::<_, String>(5)?).unwrap(),
            })
        })?;
        rows.collect()
    }

    pub fn recent_events(&self, limit: u32) -> rusqlite::Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, source, kind, subject_id, metadata FROM events
             ORDER BY timestamp DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            let subject_id_str: String = row.get(4)?;
            Ok(Event {
                id: EventId(row.get::<_, String>(0)?.parse().unwrap()),
                timestamp: row.get(1)?,
                source: serde_json::from_value(serde_json::Value::String(row.get(2)?)).unwrap(),
                kind: serde_json::from_value(serde_json::Value::String(row.get(3)?)).unwrap(),
                subject: EntityRef {
                    kind: EntityKind::File,
                    identity: subject_id_str,
                    attributes: HashMap::new(),
                },
                context: vec![],
                metadata: serde_json::from_str(&row.get::<_, String>(5)?).unwrap(),
            })
        })?;
        rows.collect()
    }

    pub fn event_count(&self) -> rusqlite::Result<u64> {
        self.conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
    }

    // === Edges ===

    pub fn insert_edge(&self, edge: &Edge) -> rusqlite::Result<()> {
        let relation_str = serde_json::to_value(&edge.relation)
            .unwrap()
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        self.conn.execute(
            "INSERT INTO edges (id, from_id, to_id, relation, strength, created_at, last_reinforced)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                 strength = excluded.strength,
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

    pub fn find_edge(&self, from: &EntityId, to: &EntityId, relation: &Relation) -> rusqlite::Result<Option<Edge>> {
        let relation_str = serde_json::to_value(relation)
            .unwrap()
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        self.conn
            .query_row(
                "SELECT id, from_id, to_id, relation, strength, created_at, last_reinforced
                 FROM edges WHERE from_id = ?1 AND to_id = ?2 AND relation = ?3",
                params![from.to_string(), to.to_string(), relation_str],
                |row| {
                    Ok(Edge {
                        id: EdgeId(row.get::<_, String>(0)?.parse().unwrap()),
                        from: EntityId(row.get::<_, String>(1)?.parse().unwrap()),
                        to: EntityId(row.get::<_, String>(2)?.parse().unwrap()),
                        relation: serde_json::from_value(serde_json::Value::String(row.get(3)?)).unwrap(),
                        strength: row.get(4)?,
                        created_at: row.get(5)?,
                        last_reinforced: row.get(6)?,
                    })
                },
            )
            .optional()
    }

    pub fn edges_from(&self, entity_id: &EntityId) -> rusqlite::Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, from_id, to_id, relation, strength, created_at, last_reinforced
             FROM edges WHERE from_id = ?1",
        )?;
        let rows = stmt.query_map(params![entity_id.to_string()], |row| {
            Ok(Edge {
                id: EdgeId(row.get::<_, String>(0)?.parse().unwrap()),
                from: EntityId(row.get::<_, String>(1)?.parse().unwrap()),
                to: EntityId(row.get::<_, String>(2)?.parse().unwrap()),
                relation: serde_json::from_value(serde_json::Value::String(row.get(3)?)).unwrap(),
                strength: row.get(4)?,
                created_at: row.get(5)?,
                last_reinforced: row.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn all_edges(&self) -> rusqlite::Result<Vec<Edge>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, from_id, to_id, relation, strength, created_at, last_reinforced FROM edges",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Edge {
                id: EdgeId(row.get::<_, String>(0)?.parse().unwrap()),
                from: EntityId(row.get::<_, String>(1)?.parse().unwrap()),
                to: EntityId(row.get::<_, String>(2)?.parse().unwrap()),
                relation: serde_json::from_value(serde_json::Value::String(row.get(3)?)).unwrap(),
                strength: row.get(4)?,
                created_at: row.get(5)?,
                last_reinforced: row.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn edge_count(&self) -> rusqlite::Result<u64> {
        self.conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
    }

    // === FTS ===

    pub fn search_entities(&self, query: &str, limit: u32) -> rusqlite::Result<Vec<Entity>> {
        let mut stmt = self.conn.prepare(
            "SELECT e.id, e.kind, e.name, e.attributes, e.first_seen, e.last_seen
             FROM entities_fts fts
             JOIN entities e ON e.rowid = fts.rowid
             WHERE entities_fts MATCH ?1
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit], |row| {
            Ok(Entity {
                id: EntityId(row.get::<_, String>(0)?.parse().unwrap()),
                kind: serde_json::from_value(serde_json::Value::String(row.get(1)?)).unwrap(),
                name: row.get(2)?,
                attributes: serde_json::from_str(&row.get::<_, String>(3)?).unwrap(),
                first_seen: row.get(4)?,
                last_seen: row.get(5)?,
            })
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrations::run_migrations;

    fn test_repo() -> Repository {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        Repository::new(conn)
    }

    fn make_entity(name: &str, kind: EntityKind) -> Entity {
        Entity {
            id: EntityId::new(),
            kind,
            name: name.to_string(),
            attributes: HashMap::new(),
            first_seen: 1000,
            last_seen: 2000,
        }
    }

    #[test]
    fn insert_and_get_entity() {
        let repo = test_repo();
        let entity = make_entity("main.rs", EntityKind::File);
        repo.insert_entity(&entity).unwrap();
        let found = repo.get_entity(&entity.id).unwrap().unwrap();
        assert_eq!(found.name, "main.rs");
    }

    #[test]
    fn insert_entity_upserts_last_seen() {
        let repo = test_repo();
        let mut entity = make_entity("main.rs", EntityKind::File);
        repo.insert_entity(&entity).unwrap();
        entity.last_seen = 9999;
        repo.insert_entity(&entity).unwrap();
        let found = repo.get_entity(&entity.id).unwrap().unwrap();
        assert_eq!(found.last_seen, 9999);
    }

    #[test]
    fn find_entity_by_kind_and_name() {
        let repo = test_repo();
        let entity = make_entity("payment-service", EntityKind::Project);
        repo.insert_entity(&entity).unwrap();
        let found = repo.find_entity_by_kind_and_name(&EntityKind::Project, "payment-service").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, entity.id);
    }

    #[test]
    fn insert_and_get_edge() {
        let repo = test_repo();
        let e1 = make_entity("file.rs", EntityKind::File);
        let e2 = make_entity("my-project", EntityKind::Project);
        repo.insert_entity(&e1).unwrap();
        repo.insert_entity(&e2).unwrap();
        let edge = Edge {
            id: EdgeId::new(),
            from: e1.id,
            to: e2.id,
            relation: Relation::BelongsTo,
            strength: 0.5,
            created_at: 1000,
            last_reinforced: 1000,
        };
        repo.insert_edge(&edge).unwrap();
        let found = repo.find_edge(&e1.id, &e2.id, &Relation::BelongsTo).unwrap();
        assert!(found.is_some());
    }

    #[test]
    fn insert_and_get_event() {
        let repo = test_repo();
        let entity = make_entity("main.rs", EntityKind::File);
        repo.insert_entity(&entity).unwrap();
        let event = Event {
            id: EventId::new(),
            timestamp: 5000,
            source: CollectorSource::Filesystem,
            kind: EventKind::FileModified,
            subject: EntityRef {
                kind: EntityKind::File,
                identity: "main.rs".to_string(),
                attributes: HashMap::new(),
            },
            context: vec![],
            metadata: HashMap::new(),
        };
        repo.insert_event(&event, &entity.id, &[]).unwrap();
        let recent = repo.recent_events(10).unwrap();
        assert_eq!(recent.len(), 1);
    }

    #[test]
    fn entity_count_works() {
        let repo = test_repo();
        assert_eq!(repo.entity_count().unwrap(), 0);
        repo.insert_entity(&make_entity("a", EntityKind::File)).unwrap();
        repo.insert_entity(&make_entity("b", EntityKind::File)).unwrap();
        assert_eq!(repo.entity_count().unwrap(), 2);
    }
}
```

**Step 4: Implement `crates/cronos-core/src/storage/mod.rs`**

```rust
pub mod migrations;
pub mod repo;

pub use repo::Repository;
```

**Step 5: Implement `crates/cronos-core/src/lib.rs`**

```rust
pub mod storage;
```

**Step 6: Run tests**

Run: `cargo test -p cronos-core`
Expected: All tests pass.

**Step 7: Commit**

```bash
git add Cargo.toml crates/cronos-core/
git commit -m "feat: add cronos-core with SQLite storage layer and migrations"
```

---

## Task 5: cronos-core/graph (In-Memory Graph)

Petgraph-based in-memory context graph, rebuilt from SQLite on startup.

**Files:**
- Create: `crates/cronos-core/src/graph/mod.rs`
- Modify: `crates/cronos-core/src/lib.rs`

**Step 1: Implement `crates/cronos-core/src/graph/mod.rs`**

```rust
use cronos_model::*;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;

pub struct ContextGraph {
    graph: DiGraph<EntityId, EdgeInfo>,
    entity_index: HashMap<EntityId, NodeIndex>,
}

#[derive(Debug, Clone)]
pub struct EdgeInfo {
    pub edge_id: EdgeId,
    pub relation: Relation,
    pub strength: f32,
}

impl ContextGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            entity_index: HashMap::new(),
        }
    }

    /// Rebuild graph from storage data
    pub fn rebuild(entities: &[Entity], edges: &[Edge]) -> Self {
        let mut g = Self::new();
        for entity in entities {
            g.add_entity(entity.id);
        }
        for edge in edges {
            g.add_edge(edge);
        }
        g
    }

    pub fn add_entity(&mut self, id: EntityId) -> NodeIndex {
        if let Some(&idx) = self.entity_index.get(&id) {
            return idx;
        }
        let idx = self.graph.add_node(id);
        self.entity_index.insert(id, idx);
        idx
    }

    pub fn add_edge(&mut self, edge: &Edge) {
        let from_idx = self.add_entity(edge.from);
        let to_idx = self.add_entity(edge.to);
        // Check if edge already exists and update
        let existing = self.graph.edges_connecting(from_idx, to_idx)
            .find(|e| e.weight().edge_id == edge.id);
        if let Some(e) = existing {
            let idx = e.id();
            if let Some(w) = self.graph.edge_weight_mut(idx) {
                w.strength = edge.strength;
            }
        } else {
            self.graph.add_edge(from_idx, to_idx, EdgeInfo {
                edge_id: edge.id,
                relation: edge.relation.clone(),
                strength: edge.strength,
            });
        }
    }

    /// Get all entities connected to the given entity within `depth` hops
    pub fn related(&self, entity_id: &EntityId, depth: u8) -> Vec<EntityId> {
        let Some(&start) = self.entity_index.get(entity_id) else {
            return vec![];
        };

        let mut visited = HashMap::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((start, 0u8));
        visited.insert(start, 0u8);

        while let Some((node, d)) = queue.pop_front() {
            if d >= depth {
                continue;
            }
            // Outgoing edges
            for neighbor in self.graph.neighbors_directed(node, petgraph::Direction::Outgoing) {
                if !visited.contains_key(&neighbor) {
                    visited.insert(neighbor, d + 1);
                    queue.push_back((neighbor, d + 1));
                }
            }
            // Incoming edges
            for neighbor in self.graph.neighbors_directed(node, petgraph::Direction::Incoming) {
                if !visited.contains_key(&neighbor) {
                    visited.insert(neighbor, d + 1);
                    queue.push_back((neighbor, d + 1));
                }
            }
        }

        visited.into_iter()
            .filter(|(idx, _)| *idx != start)
            .map(|(idx, _)| self.graph[idx])
            .collect()
    }

    pub fn entity_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    pub fn has_entity(&self, id: &EntityId) -> bool {
        self.entity_index.contains_key(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_query_entities() {
        let mut g = ContextGraph::new();
        let id1 = EntityId::new();
        let id2 = EntityId::new();
        g.add_entity(id1);
        g.add_entity(id2);
        assert_eq!(g.entity_count(), 2);
        assert!(g.has_entity(&id1));
    }

    #[test]
    fn add_entity_is_idempotent() {
        let mut g = ContextGraph::new();
        let id = EntityId::new();
        g.add_entity(id);
        g.add_entity(id);
        assert_eq!(g.entity_count(), 1);
    }

    #[test]
    fn related_traversal() {
        let mut g = ContextGraph::new();
        let file = EntityId::new();
        let project = EntityId::new();
        let repo = EntityId::new();

        let edge1 = Edge {
            id: EdgeId::new(), from: file, to: project,
            relation: Relation::BelongsTo, strength: 0.8,
            created_at: 1000, last_reinforced: 1000,
        };
        let edge2 = Edge {
            id: EdgeId::new(), from: project, to: repo,
            relation: Relation::Contains, strength: 0.9,
            created_at: 1000, last_reinforced: 1000,
        };
        g.add_edge(&edge1);
        g.add_edge(&edge2);

        // Depth 1: file -> project
        let related = g.related(&file, 1);
        assert_eq!(related.len(), 1);
        assert!(related.contains(&project));

        // Depth 2: file -> project -> repo
        let related = g.related(&file, 2);
        assert_eq!(related.len(), 2);
        assert!(related.contains(&project));
        assert!(related.contains(&repo));
    }

    #[test]
    fn rebuild_from_data() {
        let e1 = Entity {
            id: EntityId::new(), kind: EntityKind::File,
            name: "a".into(), attributes: Default::default(),
            first_seen: 0, last_seen: 0,
        };
        let e2 = Entity {
            id: EntityId::new(), kind: EntityKind::Project,
            name: "b".into(), attributes: Default::default(),
            first_seen: 0, last_seen: 0,
        };
        let edge = Edge {
            id: EdgeId::new(), from: e1.id, to: e2.id,
            relation: Relation::BelongsTo, strength: 0.5,
            created_at: 0, last_reinforced: 0,
        };
        let g = ContextGraph::rebuild(&[e1, e2], &[edge]);
        assert_eq!(g.entity_count(), 2);
        assert_eq!(g.edge_count(), 1);
    }
}
```

**Step 2: Update `crates/cronos-core/src/lib.rs`**

```rust
pub mod graph;
pub mod storage;
```

**Step 3: Run tests**

Run: `cargo test -p cronos-core`
Expected: All tests pass (storage + graph).

**Step 4: Commit**

```bash
git add crates/cronos-core/src/graph/
git commit -m "feat: add in-memory context graph with petgraph"
```

---

## Task 6: cronos-core/ingest + linker

Event normalization, dedup, entity resolution, edge creation.

**Files:**
- Create: `crates/cronos-core/src/ingest/mod.rs`
- Create: `crates/cronos-core/src/linker/mod.rs`
- Modify: `crates/cronos-core/src/lib.rs`

**Step 1: Implement `crates/cronos-core/src/ingest/mod.rs`**

```rust
use cronos_model::*;
use std::collections::HashMap;

pub struct IngestPipeline {
    /// (source, subject_identity) -> last timestamp
    dedup_cache: HashMap<(String, String), Timestamp>,
    dedup_window_ms: u64,
}

impl IngestPipeline {
    pub fn new(dedup_window_ms: u64) -> Self {
        Self {
            dedup_cache: HashMap::new(),
            dedup_window_ms,
        }
    }

    /// Returns None if the event is a duplicate, Some(event) if it should be processed
    pub fn process(&mut self, event: Event) -> Option<Event> {
        // Validate
        if event.subject.identity.is_empty() {
            tracing::warn!(event_id = %event.id, "dropping event with empty subject identity");
            return None;
        }

        // Dedup: same source + subject within window
        let source_str = format!("{:?}", event.source);
        let key = (source_str, event.subject.identity.clone());
        if let Some(&last_ts) = self.dedup_cache.get(&key) {
            if (event.timestamp - last_ts).unsigned_abs() < self.dedup_window_ms {
                tracing::debug!(event_id = %event.id, "deduplicating event");
                return None;
            }
        }
        self.dedup_cache.insert(key, event.timestamp);

        Some(event)
    }

    /// Prune old dedup cache entries to prevent unbounded growth
    pub fn prune_cache(&mut self, before: Timestamp) {
        self.dedup_cache.retain(|_, ts| *ts >= before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_event(identity: &str, timestamp: Timestamp) -> Event {
        Event {
            id: EventId::new(),
            timestamp,
            source: CollectorSource::Filesystem,
            kind: EventKind::FileModified,
            subject: EntityRef {
                kind: EntityKind::File,
                identity: identity.to_string(),
                attributes: HashMap::new(),
            },
            context: vec![],
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn passes_valid_event() {
        let mut pipeline = IngestPipeline::new(1000);
        let event = make_event("/src/main.rs", 5000);
        assert!(pipeline.process(event).is_some());
    }

    #[test]
    fn drops_empty_identity() {
        let mut pipeline = IngestPipeline::new(1000);
        let event = make_event("", 5000);
        assert!(pipeline.process(event).is_none());
    }

    #[test]
    fn deduplicates_within_window() {
        let mut pipeline = IngestPipeline::new(1000);
        let e1 = make_event("/src/main.rs", 5000);
        let e2 = make_event("/src/main.rs", 5500); // within 1000ms
        assert!(pipeline.process(e1).is_some());
        assert!(pipeline.process(e2).is_none());
    }

    #[test]
    fn allows_after_window() {
        let mut pipeline = IngestPipeline::new(1000);
        let e1 = make_event("/src/main.rs", 5000);
        let e2 = make_event("/src/main.rs", 6500); // after 1000ms
        assert!(pipeline.process(e1).is_some());
        assert!(pipeline.process(e2).is_some());
    }

    #[test]
    fn different_files_not_deduped() {
        let mut pipeline = IngestPipeline::new(1000);
        let e1 = make_event("/src/main.rs", 5000);
        let e2 = make_event("/src/lib.rs", 5000);
        assert!(pipeline.process(e1).is_some());
        assert!(pipeline.process(e2).is_some());
    }
}
```

**Step 2: Implement `crates/cronos-core/src/linker/mod.rs`**

```rust
use cronos_model::*;
use crate::graph::ContextGraph;
use crate::storage::Repository;

pub struct Linker {
    temporal_window_ms: u64,
}

impl Linker {
    pub fn new(temporal_window_ms: u64) -> Self {
        Self { temporal_window_ms }
    }

    /// Resolve an EntityRef to an Entity. Find existing or create new.
    pub fn resolve_entity_ref(
        &self,
        entity_ref: &EntityRef,
        timestamp: Timestamp,
        repo: &Repository,
        graph: &mut ContextGraph,
    ) -> rusqlite::Result<Entity> {
        // Try to find existing entity by kind + identity
        if let Some(mut existing) = repo.find_entity_by_kind_and_name(&entity_ref.kind, &entity_ref.identity)? {
            // Update last_seen
            existing.last_seen = timestamp;
            repo.insert_entity(&existing)?;
            graph.add_entity(existing.id);
            return Ok(existing);
        }

        // Create new entity
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

    /// Process a fully validated event: resolve entities, create edges, store everything
    pub fn link(
        &self,
        event: &Event,
        repo: &Repository,
        graph: &mut ContextGraph,
    ) -> rusqlite::Result<()> {
        let now = event.timestamp;

        // Resolve subject entity
        let subject = self.resolve_entity_ref(&event.subject, now, repo, graph)?;

        // Resolve context entities
        let mut context_ids = Vec::new();
        for ctx_ref in &event.context {
            let ctx_entity = self.resolve_entity_ref(ctx_ref, now, repo, graph)?;
            context_ids.push(ctx_entity.id);

            // Create edge: subject -> context entity
            let relation = infer_relation(&event.subject.kind, &ctx_ref.kind);
            self.ensure_edge(subject.id, ctx_entity.id, relation, now, repo, graph)?;
        }

        // Store the event
        repo.insert_event(event, &subject.id, &context_ids)?;

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
        if let Some(mut existing) = repo.find_edge(&from, &to, &relation)? {
            // Reinforce existing edge
            existing.strength = (existing.strength + 0.1).min(1.0);
            existing.last_reinforced = timestamp;
            repo.insert_edge(&existing)?;
            graph.add_edge(&existing);
        } else {
            // Create new edge
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

/// Infer the relationship type based on entity kinds
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
    use crate::storage::migrations::run_migrations;
    use rusqlite::Connection;
    use std::collections::HashMap;

    fn setup() -> (Repository, ContextGraph, Linker) {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        let repo = Repository::new(conn);
        let graph = ContextGraph::new();
        let linker = Linker::new(300_000);
        (repo, graph, linker)
    }

    #[test]
    fn resolve_creates_new_entity() {
        let (repo, mut graph, linker) = setup();
        let entity_ref = EntityRef {
            kind: EntityKind::File,
            identity: "/src/main.rs".to_string(),
            attributes: HashMap::new(),
        };
        let entity = linker.resolve_entity_ref(&entity_ref, 1000, &repo, &mut graph).unwrap();
        assert_eq!(entity.name, "/src/main.rs");
        assert_eq!(entity.kind, EntityKind::File);
        assert!(graph.has_entity(&entity.id));
    }

    #[test]
    fn resolve_finds_existing_entity() {
        let (repo, mut graph, linker) = setup();
        let entity_ref = EntityRef {
            kind: EntityKind::File,
            identity: "/src/main.rs".to_string(),
            attributes: HashMap::new(),
        };
        let e1 = linker.resolve_entity_ref(&entity_ref, 1000, &repo, &mut graph).unwrap();
        let e2 = linker.resolve_entity_ref(&entity_ref, 2000, &repo, &mut graph).unwrap();
        assert_eq!(e1.id, e2.id); // same entity
        assert_eq!(e2.last_seen, 2000); // updated
    }

    #[test]
    fn link_creates_entities_and_edges() {
        let (repo, mut graph, linker) = setup();
        let event = Event {
            id: EventId::new(),
            timestamp: 5000,
            source: CollectorSource::Filesystem,
            kind: EventKind::FileModified,
            subject: EntityRef {
                kind: EntityKind::File,
                identity: "/projects/foo/main.rs".to_string(),
                attributes: HashMap::new(),
            },
            context: vec![EntityRef {
                kind: EntityKind::Project,
                identity: "/projects/foo".to_string(),
                attributes: HashMap::new(),
            }],
            metadata: HashMap::new(),
        };
        linker.link(&event, &repo, &mut graph).unwrap();
        assert_eq!(repo.entity_count().unwrap(), 2);
        assert_eq!(repo.event_count().unwrap(), 1);
        assert_eq!(repo.edge_count().unwrap(), 1);
        assert_eq!(graph.entity_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn repeated_events_reinforce_edges() {
        let (repo, mut graph, linker) = setup();
        let make = |ts| Event {
            id: EventId::new(),
            timestamp: ts,
            source: CollectorSource::Filesystem,
            kind: EventKind::FileModified,
            subject: EntityRef {
                kind: EntityKind::File,
                identity: "/projects/foo/main.rs".to_string(),
                attributes: HashMap::new(),
            },
            context: vec![EntityRef {
                kind: EntityKind::Project,
                identity: "/projects/foo".to_string(),
                attributes: HashMap::new(),
            }],
            metadata: HashMap::new(),
        };
        linker.link(&make(1000), &repo, &mut graph).unwrap();
        linker.link(&make(2000), &repo, &mut graph).unwrap();
        let edges = repo.all_edges().unwrap();
        assert_eq!(edges.len(), 1);
        assert!(edges[0].strength > 0.5); // reinforced
    }

    #[test]
    fn infer_relation_file_to_project() {
        assert_eq!(
            infer_relation(&EntityKind::File, &EntityKind::Project),
            Relation::BelongsTo
        );
    }
}
```

**Step 3: Update `crates/cronos-core/src/lib.rs`**

```rust
pub mod graph;
pub mod ingest;
pub mod linker;
pub mod storage;
```

**Step 4: Run tests**

Run: `cargo test -p cronos-core`
Expected: All tests pass.

**Step 5: Commit**

```bash
git add crates/cronos-core/src/ingest/ crates/cronos-core/src/linker/ crates/cronos-core/src/lib.rs
git commit -m "feat: add ingest pipeline and linker for entity resolution and edge creation"
```

---

## Task 7: cronos-core/server (Unix Socket Server)

Tokio-based Unix socket server that accepts collector and CLI connections.

**Files:**
- Create: `crates/cronos-core/src/server/mod.rs`
- Create: `crates/cronos-core/src/engine.rs`
- Modify: `crates/cronos-core/src/lib.rs`

**Step 1: Implement `crates/cronos-core/src/engine.rs`**

The engine owns the graph, repo, ingest, and linker. Shared state behind Arc<Mutex>.

```rust
use crate::graph::ContextGraph;
use crate::ingest::IngestPipeline;
use crate::linker::Linker;
use crate::storage::{migrations, Repository};
use cronos_common::config::DaemonConfig;
use cronos_model::*;
use cronos_proto::*;
use rusqlite::Connection;
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
        let conn = Connection::open(db_path)?;
        migrations::run_migrations(&conn)?;
        let repo = Repository::new(conn);

        // Rebuild in-memory graph
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
            MessageKind::CollectorHandshake { name, collector_version, source } => {
                self.handle_handshake(request_id, name, collector_version, source)
            }
            MessageKind::Heartbeat => Message::ack(request_id),
            MessageKind::Query { query } => self.handle_query(request_id, query),
            MessageKind::Status => self.handle_status(request_id),
            MessageKind::ListCollectors => self.handle_list_collectors(request_id),
            _ => Message::error(request_id, ErrorCode::BadRequest, "unexpected message type"),
        }
    }

    fn handle_emit_event(&self, request_id: String, event: Event) -> Message {
        // Ingest pipeline
        let processed = {
            let mut ingest = self.ingest.lock().unwrap();
            ingest.process(event)
        };

        let Some(event) = processed else {
            return Message::ack(request_id); // deduped, still ack
        };

        // Link
        let result = {
            let repo = self.repo.lock().unwrap();
            let mut graph = self.graph.lock().unwrap();
            self.linker.link(&event, &repo, &mut graph)
        };

        match result {
            Ok(()) => {
                // Update collector stats
                let source_str = format!("{:?}", event.source);
                if let Some(info) = self.collectors.lock().unwrap().values_mut()
                    .find(|c| format!("{:?}", c.source) == source_str)
                {
                    info.events_sent += 1;
                }
                Message::ack(request_id)
            }
            Err(e) => Message::error(request_id, ErrorCode::InternalError, e.to_string()),
        }
    }

    fn handle_handshake(&self, request_id: String, name: String, _version: String, source: CollectorSource) -> Message {
        let now = cronos_common::now_ms();
        self.collectors.lock().unwrap().insert(name.clone(), CollectorInfo {
            name,
            source,
            connected: true,
            last_heartbeat: Some(now),
            events_sent: 0,
        });
        Message::ack(request_id)
    }

    fn handle_query(&self, request_id: String, query: QueryRequest) -> Message {
        let result = match query.kind {
            QueryKind::Search { text, limit } => {
                let repo = self.repo.lock().unwrap();
                repo.search_entities(&text, limit)
                    .map(|entities| QueryResponse { entities, edges: vec![], events: vec![] })
            }
            QueryKind::Recent { limit } => {
                let repo = self.repo.lock().unwrap();
                repo.recent_events(limit)
                    .map(|events| QueryResponse { entities: vec![], edges: vec![], events })
            }
            QueryKind::Timeline { from, to } => {
                let repo = self.repo.lock().unwrap();
                repo.events_in_range(from, to)
                    .map(|events| QueryResponse { entities: vec![], edges: vec![], events })
            }
            QueryKind::Related { entity_id, depth } => {
                let graph = self.graph.lock().unwrap();
                let related_ids = graph.related(&entity_id, depth);
                let repo = self.repo.lock().unwrap();
                let mut entities = Vec::new();
                for id in &related_ids {
                    if let Ok(Some(e)) = repo.get_entity(id) {
                        entities.push(e);
                    }
                }
                Ok(QueryResponse { entities, edges: vec![], events: vec![] })
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
            entity_count: repo.entity_count().unwrap_or(0),
            edge_count: repo.edge_count().unwrap_or(0),
            event_count: repo.event_count().unwrap_or(0),
            connected_collectors: self.collectors.lock().unwrap().len() as u32,
        };
        Message::new(request_id, MessageKind::StatusResult { info })
    }

    fn handle_list_collectors(&self, request_id: String) -> Message {
        let collectors: Vec<CollectorInfo> = self.collectors.lock().unwrap().values().cloned().collect();
        Message::new(request_id, MessageKind::CollectorList { collectors })
    }
}
```

**Step 2: Implement `crates/cronos-core/src/server/mod.rs`**

```rust
use crate::engine::Engine;
use cronos_proto::{read_frame, write_frame};
use std::path::Path;
use std::sync::Arc;
use tokio::net::UnixListener;

pub async fn run(engine: Arc<Engine>, socket_path: &Path) -> anyhow::Result<()> {
    // Ensure parent dir exists
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Remove stale socket
    let _ = tokio::fs::remove_file(socket_path).await;

    let listener = UnixListener::bind(socket_path)?;
    tracing::info!(path = %socket_path.display(), "listening on unix socket");

    loop {
        let (stream, _addr) = listener.accept().await?;
        let engine = Arc::clone(&engine);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(engine, stream).await {
                tracing::debug!("connection closed: {}", e);
            }
        });
    }
}

async fn handle_connection(engine: Arc<Engine>, stream: tokio::net::UnixStream) -> anyhow::Result<()> {
    let (mut reader, mut writer) = stream.into_split();

    loop {
        let msg = read_frame(&mut reader).await?;
        let response = engine.handle_message(msg);
        write_frame(&mut writer, &response).await?;
    }
}
```

**Step 3: Update `crates/cronos-core/src/lib.rs`**

```rust
pub mod engine;
pub mod graph;
pub mod ingest;
pub mod linker;
pub mod server;
pub mod storage;
```

**Step 4: Run tests**

Run: `cargo test -p cronos-core`
Expected: All tests pass. (Server tests are integration-level and covered in Task 10.)

**Step 5: Commit**

```bash
git add crates/cronos-core/src/engine.rs crates/cronos-core/src/server/ crates/cronos-core/src/lib.rs
git commit -m "feat: add engine and unix socket server for event ingestion and queries"
```

---

## Task 8: cronos CLI Binary

Single binary with subcommands: daemon, status, query, timeline, recent, init.

**Files:**
- Create: `crates/cronos/Cargo.toml`
- Create: `crates/cronos/src/main.rs`
- Modify: `Cargo.toml` (add to workspace members)

**Step 1: Create Cargo.toml**

```toml
# crates/cronos/Cargo.toml
[package]
name = "cronos"
version.workspace = true
edition.workspace = true

[[bin]]
name = "cronos"
path = "src/main.rs"

[dependencies]
cronos-core = { path = "../cronos-core" }
cronos-common = { path = "../cronos-common" }
cronos-model = { path = "../cronos-model" }
cronos-proto = { path = "../cronos-proto" }
clap = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
anyhow = { workspace = true }
serde_json = { workspace = true }
ulid = { workspace = true }
```

**Step 2: Implement `crates/cronos/src/main.rs`**

```rust
use anyhow::Result;
use clap::{Parser, Subcommand};
use cronos_common::{CronosConfig, CronosPaths};
use cronos_core::engine::Engine;
use cronos_proto::*;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "cronos", about = "Local-first context engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Log level override
    #[arg(long, global = true)]
    log_level: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the daemon (foreground)
    Daemon,

    /// Initialize default config file
    Init,

    /// Show daemon status
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Query the context graph
    Query {
        /// Search text
        text: Option<String>,

        /// Show entities related to this entity ID
        #[arg(long)]
        related: Option<String>,

        /// Traversal depth for --related
        #[arg(long, default_value = "2")]
        depth: u8,

        /// Max results
        #[arg(long, default_value = "20")]
        limit: u32,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show recent events
    Recent {
        /// Max events to show
        #[arg(long, default_value = "20")]
        limit: u32,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show activity timeline
    Timeline {
        /// Start time (ISO 8601 or relative like "2h")
        #[arg(long)]
        from: Option<String>,

        /// End time (ISO 8601)
        #[arg(long)]
        to: Option<String>,

        /// Shorthand for relative time window (e.g. "2h", "30m")
        #[arg(long)]
        last: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show resolved config
    #[command(name = "config")]
    ConfigShow,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = CronosPaths::resolve()?;
    let config = CronosConfig::load(&paths.config_file)?;

    let log_level = cli.log_level.as_deref().unwrap_or(&config.daemon.log_level);
    cronos_common::init_tracing(log_level);

    match cli.command {
        Commands::Daemon => cmd_daemon(config, paths).await,
        Commands::Init => cmd_init(paths),
        Commands::Status { json } => cmd_client_request(
            &paths,
            Message::new("status", MessageKind::Status),
            json,
        ).await,
        Commands::Query { text, related, depth, limit, json } => {
            let query = if let Some(id_str) = related {
                let ulid: ulid::Ulid = id_str.parse()
                    .map_err(|e| anyhow::anyhow!("invalid entity ID: {}", e))?;
                QueryKind::Related { entity_id: cronos_model::EntityId(ulid), depth }
            } else if let Some(text) = text {
                QueryKind::Search { text, limit }
            } else {
                anyhow::bail!("provide search text or --related <id>");
            };
            cmd_client_request(
                &paths,
                Message::new("query", MessageKind::Query { query: QueryRequest { kind: query } }),
                json,
            ).await
        }
        Commands::Recent { limit, json } => {
            cmd_client_request(
                &paths,
                Message::new("recent", MessageKind::Query {
                    query: QueryRequest { kind: QueryKind::Recent { limit } },
                }),
                json,
            ).await
        }
        Commands::Timeline { last, from, to, json } => {
            let now = cronos_common::now_ms();
            let (from_ts, to_ts) = if let Some(last) = last {
                let ms = parse_duration_ms(&last)?;
                (now - ms, now)
            } else {
                let from_ts = from.map(|s| parse_timestamp(&s)).transpose()?.unwrap_or(now - 86_400_000);
                let to_ts = to.map(|s| parse_timestamp(&s)).transpose()?.unwrap_or(now);
                (from_ts, to_ts)
            };
            cmd_client_request(
                &paths,
                Message::new("timeline", MessageKind::Query {
                    query: QueryRequest { kind: QueryKind::Timeline { from: from_ts, to: to_ts } },
                }),
                json,
            ).await
        }
        Commands::ConfigShow => {
            let toml_str = toml::to_string_pretty(&config)?;
            println!("{}", toml_str);
            Ok(())
        }
    }
}

async fn cmd_daemon(config: CronosConfig, paths: CronosPaths) -> Result<()> {
    let db_path = if config.daemon.db_path.is_empty() {
        paths.db_file.clone()
    } else {
        PathBuf::from(&config.daemon.db_path)
    };

    let socket_path = if config.daemon.socket_path.is_empty() {
        paths.socket_file.clone()
    } else {
        PathBuf::from(&config.daemon.socket_path)
    };

    tracing::info!(db = %db_path.display(), socket = %socket_path.display(), "starting cronos daemon");

    let engine = Arc::new(Engine::open(&db_path, &config.daemon)?);

    // Handle shutdown signals
    let engine_ref = Arc::clone(&engine);
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("shutting down...");
        drop(engine_ref);
        std::process::exit(0);
    });

    cronos_core::server::run(engine, &socket_path).await
}

fn cmd_init(paths: CronosPaths) -> Result<()> {
    std::fs::create_dir_all(&paths.config_dir)?;
    if paths.config_file.exists() {
        println!("Config already exists: {}", paths.config_file.display());
    } else {
        let default = include_str!("../../../config/cronos.default.toml");
        std::fs::write(&paths.config_file, default)?;
        println!("Created config: {}", paths.config_file.display());
    }
    Ok(())
}

async fn cmd_client_request(paths: &CronosPaths, msg: Message, json: bool) -> Result<()> {
    let stream = tokio::net::UnixStream::connect(&paths.socket_file).await
        .map_err(|_| anyhow::anyhow!("cannot connect to daemon at {}. Is it running?", paths.socket_file.display()))?;
    let (mut reader, mut writer) = stream.into_split();
    cronos_proto::write_frame(&mut writer, &msg).await?;
    let response = cronos_proto::read_frame(&mut reader).await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        print_response(&response);
    }
    Ok(())
}

fn print_response(msg: &Message) {
    match &msg.kind {
        MessageKind::StatusResult { info } => {
            println!("Cronos daemon status:");
            println!("  Uptime: {}s", info.uptime_secs);
            println!("  Entities: {}", info.entity_count);
            println!("  Edges: {}", info.edge_count);
            println!("  Events: {}", info.event_count);
            println!("  Connected collectors: {}", info.connected_collectors);
        }
        MessageKind::QueryResult { response } => {
            if !response.entities.is_empty() {
                println!("Entities:");
                for e in &response.entities {
                    println!("  [{}] {} ({})", e.id, e.name, e.kind);
                }
            }
            if !response.events.is_empty() {
                println!("Events:");
                for e in &response.events {
                    println!("  [{}] {:?} {:?} -> {}", e.timestamp, e.source, e.kind, e.subject.identity);
                }
            }
            if response.entities.is_empty() && response.events.is_empty() {
                println!("No results.");
            }
        }
        MessageKind::CollectorList { collectors } => {
            println!("Collectors:");
            for c in collectors {
                let status = if c.connected { "connected" } else { "disconnected" };
                println!("  {} ({:?}) - {} - {} events", c.name, c.source, status, c.events_sent);
            }
        }
        MessageKind::Error { message, .. } => {
            eprintln!("Error: {}", message);
        }
        _ => {
            println!("{:?}", msg.kind);
        }
    }
}

fn parse_duration_ms(s: &str) -> Result<i64> {
    let s = s.trim();
    if let Some(h) = s.strip_suffix('h') {
        Ok(h.parse::<i64>()? * 3_600_000)
    } else if let Some(m) = s.strip_suffix('m') {
        Ok(m.parse::<i64>()? * 60_000)
    } else if let Some(s_val) = s.strip_suffix('s') {
        Ok(s_val.parse::<i64>()? * 1_000)
    } else {
        anyhow::bail!("invalid duration: '{}'. Use format like '2h', '30m', '60s'", s);
    }
}

fn parse_timestamp(s: &str) -> Result<i64> {
    // Try ISO 8601
    if let Ok(dt) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(dt.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp_millis());
    }
    anyhow::bail!("invalid timestamp: '{}'. Use YYYY-MM-DD or RFC 3339", s);
}
```

Add `chrono = { workspace = true }` and `toml = { workspace = true }` to cronos binary dependencies.

**Step 3: Build and verify**

Run: `cargo build -p cronos`
Expected: Compiles successfully.

Run: `cargo run -p cronos -- --help`
Expected: Shows help text with subcommands.

**Step 4: Commit**

```bash
git add Cargo.toml crates/cronos/
git commit -m "feat: add cronos CLI binary with daemon and query subcommands"
```

---

## Task 9: cronos-collect-fs (Filesystem Collector)

Inotify-based file watcher that emits events to the daemon.

**Files:**
- Create: `crates/collectors/cronos-collect-fs/Cargo.toml`
- Create: `crates/collectors/cronos-collect-fs/src/main.rs`
- Modify: `Cargo.toml` (add to workspace members)

**Step 1: Create Cargo.toml**

```toml
[package]
name = "cronos-collect-fs"
version.workspace = true
edition.workspace = true

[[bin]]
name = "cronos-collect-fs"
path = "src/main.rs"

[dependencies]
cronos-model = { path = "../../cronos-model" }
cronos-proto = { path = "../../cronos-proto" }
cronos-common = { path = "../../cronos-common" }
tokio = { workspace = true }
notify = { workspace = true }
tracing = { workspace = true }
anyhow = { workspace = true }
serde_json = { workspace = true }
ulid = { workspace = true }
glob-match = "0.2"
```

Add `glob-match = "0.2"` to `[workspace.dependencies]` in root Cargo.toml.

**Step 2: Implement `crates/collectors/cronos-collect-fs/src/main.rs`**

```rust
use anyhow::Result;
use cronos_common::CronosPaths;
use cronos_model::*;
use cronos_proto::*;
use notify::{Config, Event as NotifyEvent, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let paths = CronosPaths::resolve()?;
    let config = cronos_common::CronosConfig::load(&paths.config_file)?;
    cronos_common::init_tracing(&config.daemon.log_level);

    let fs_config = &config.collectors.fs;
    if !fs_config.enabled {
        tracing::info!("filesystem collector is disabled");
        return Ok(());
    }

    // Connect to daemon
    let socket_path = if config.daemon.socket_path.is_empty() {
        paths.socket_file.clone()
    } else {
        PathBuf::from(&config.daemon.socket_path)
    };

    tracing::info!(socket = %socket_path.display(), "connecting to daemon");
    let stream = tokio::net::UnixStream::connect(&socket_path).await?;
    let (mut reader, mut writer) = stream.into_split();

    // Send handshake
    let handshake = Message::new(
        "handshake",
        MessageKind::CollectorHandshake {
            name: "fs".to_string(),
            collector_version: env!("CARGO_PKG_VERSION").to_string(),
            source: CollectorSource::Filesystem,
        },
    );
    write_frame(&mut writer, &handshake).await?;
    let ack = read_frame(&mut reader).await?;
    tracing::info!("handshake: {:?}", ack.kind);

    // Set up file watcher
    let (tx, mut rx) = mpsc::channel::<NotifyEvent>(1024);
    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<NotifyEvent>| {
            if let Ok(event) = res {
                let _ = tx.blocking_send(event);
            }
        },
        Config::default(),
    )?;

    // Watch configured paths
    let watch_paths: Vec<PathBuf> = fs_config.watch_paths.iter()
        .map(|p| {
            if p.starts_with('~') {
                PathBuf::from(p.replacen('~', &std::env::var("HOME").unwrap_or_default(), 1))
            } else {
                PathBuf::from(p)
            }
        })
        .collect();

    for path in &watch_paths {
        if path.exists() {
            watcher.watch(path, RecursiveMode::Recursive)?;
            tracing::info!(path = %path.display(), "watching directory");
        } else {
            tracing::warn!(path = %path.display(), "watch path does not exist, skipping");
        }
    }

    let ignore_patterns = fs_config.ignore_patterns.clone();

    // Process file events
    while let Some(notify_event) = rx.recv().await {
        for path in &notify_event.paths {
            let path_str = path.to_string_lossy().to_string();

            // Check ignore patterns
            if should_ignore(&path_str, &ignore_patterns) {
                continue;
            }

            let event_kind = match notify_event.kind {
                EventKind::Create(_) => EventKind::Create(notify::event::CreateKind::Any),
                EventKind::Modify(_) => EventKind::Modify(notify::event::ModifyKind::Any),
                EventKind::Remove(_) => EventKind::Remove(notify::event::RemoveKind::Any),
                _ => continue,
            };

            let cronos_kind = match event_kind {
                EventKind::Create(_) => cronos_model::EventKind::FileCreated,
                EventKind::Modify(_) => cronos_model::EventKind::FileModified,
                EventKind::Remove(_) => cronos_model::EventKind::FileDeleted,
                _ => continue,
            };

            // Detect project root
            let context = detect_project_root(path)
                .map(|root| vec![EntityRef {
                    kind: EntityKind::Project,
                    identity: root.to_string_lossy().to_string(),
                    attributes: HashMap::new(),
                }])
                .unwrap_or_default();

            let event = cronos_model::Event {
                id: EventId::new(),
                timestamp: cronos_common::now_ms(),
                source: CollectorSource::Filesystem,
                kind: cronos_kind,
                subject: EntityRef {
                    kind: EntityKind::File,
                    identity: path_str,
                    attributes: HashMap::new(),
                },
                context,
                metadata: HashMap::new(),
            };

            let msg = Message::new(
                event.id.to_string(),
                MessageKind::EmitEvent { event },
            );

            if let Err(e) = write_frame(&mut writer, &msg).await {
                tracing::error!("failed to send event: {}", e);
                break;
            }
            // Read ack (non-blocking drain)
            let _ = read_frame(&mut reader).await;
        }
    }

    Ok(())
}

fn should_ignore(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| glob_match::glob_match(pattern, path))
}

fn detect_project_root(path: &Path) -> Option<PathBuf> {
    let markers = [".git", "Cargo.toml", "package.json", "go.mod", "pyproject.toml", "Makefile"];
    let mut dir = if path.is_file() { path.parent()? } else { path };
    loop {
        for marker in &markers {
            if dir.join(marker).exists() {
                return Some(dir.to_path_buf());
            }
        }
        dir = dir.parent()?;
    }
}
```

**Step 3: Build**

Run: `cargo build -p cronos-collect-fs`
Expected: Compiles successfully.

**Step 4: Commit**

```bash
git add Cargo.toml crates/collectors/cronos-collect-fs/
git commit -m "feat: add filesystem collector with inotify watcher and project detection"
```

---

## Task 10: systemd Service Template

**Files:**
- Create: `assets/systemd/cronos.service`

**Step 1: Create service file**

```ini
[Unit]
Description=Cronos Context Engine
Documentation=https://github.com/your-user/cronos
After=default.target

[Service]
Type=simple
ExecStart=%h/.cargo/bin/cronos daemon
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
```

**Step 2: Commit**

```bash
git add assets/
git commit -m "feat: add systemd user service template"
```

---

## Task 11: Integration Test

End-to-end test: start daemon, connect a mock collector, emit events, query results.

**Files:**
- Create: `tests/integration_test.rs` (in workspace root)

**Step 1: Create test**

```rust
use cronos_common::config::DaemonConfig;
use cronos_core::engine::Engine;
use cronos_model::*;
use cronos_proto::*;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

fn make_engine(dir: &TempDir) -> Arc<Engine> {
    let db_path = dir.path().join("test.db");
    let config = DaemonConfig::default();
    Arc::new(Engine::open(&db_path, &config).unwrap())
}

#[test]
fn engine_handles_event_and_query() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    // Emit an event
    let event = Event {
        id: EventId::new(),
        timestamp: 5000,
        source: CollectorSource::Filesystem,
        kind: EventKind::FileModified,
        subject: EntityRef {
            kind: EntityKind::File,
            identity: "/projects/foo/main.rs".to_string(),
            attributes: HashMap::new(),
        },
        context: vec![EntityRef {
            kind: EntityKind::Project,
            identity: "/projects/foo".to_string(),
            attributes: HashMap::new(),
        }],
        metadata: HashMap::new(),
    };
    let msg = Message::new("e1", MessageKind::EmitEvent { event });
    let resp = engine.handle_message(msg);
    assert!(matches!(resp.kind, MessageKind::Ack { .. }));

    // Query status
    let msg = Message::new("s1", MessageKind::Status);
    let resp = engine.handle_message(msg);
    match resp.kind {
        MessageKind::StatusResult { info } => {
            assert_eq!(info.entity_count, 2); // file + project
            assert_eq!(info.edge_count, 1);   // file -> project
            assert_eq!(info.event_count, 1);
        }
        other => panic!("expected StatusResult, got {:?}", other),
    }

    // Query recent
    let msg = Message::new("r1", MessageKind::Query {
        query: QueryRequest { kind: QueryKind::Recent { limit: 10 } },
    });
    let resp = engine.handle_message(msg);
    match resp.kind {
        MessageKind::QueryResult { response } => {
            assert_eq!(response.events.len(), 1);
        }
        other => panic!("expected QueryResult, got {:?}", other),
    }
}

#[test]
fn engine_deduplicates_rapid_events() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    let make = |ts| {
        let event = Event {
            id: EventId::new(),
            timestamp: ts,
            source: CollectorSource::Filesystem,
            kind: EventKind::FileModified,
            subject: EntityRef {
                kind: EntityKind::File,
                identity: "/projects/foo/main.rs".to_string(),
                attributes: HashMap::new(),
            },
            context: vec![],
            metadata: HashMap::new(),
        };
        Message::new(event.id.to_string(), MessageKind::EmitEvent { event })
    };

    engine.handle_message(make(1000));
    engine.handle_message(make(1500)); // within 1000ms dedup window

    let resp = engine.handle_message(Message::new("s1", MessageKind::Status));
    match resp.kind {
        MessageKind::StatusResult { info } => {
            assert_eq!(info.event_count, 1); // only one event stored
        }
        other => panic!("expected StatusResult, got {:?}", other),
    }
}

#[test]
fn engine_handles_collector_handshake() {
    let dir = TempDir::new().unwrap();
    let engine = make_engine(&dir);

    let msg = Message::new("h1", MessageKind::CollectorHandshake {
        name: "fs".to_string(),
        collector_version: "0.1.0".to_string(),
        source: CollectorSource::Filesystem,
    });
    let resp = engine.handle_message(msg);
    assert!(matches!(resp.kind, MessageKind::Ack { .. }));

    // List collectors
    let resp = engine.handle_message(Message::new("l1", MessageKind::ListCollectors));
    match resp.kind {
        MessageKind::CollectorList { collectors } => {
            assert_eq!(collectors.len(), 1);
            assert_eq!(collectors[0].name, "fs");
        }
        other => panic!("expected CollectorList, got {:?}", other),
    }
}
```

**Step 2: Add test dependencies to workspace root**

In root `Cargo.toml`, add:
```toml
[dev-dependencies]
tempfile = "3"
cronos-core = { path = "crates/cronos-core" }
cronos-model = { path = "crates/cronos-model" }
cronos-proto = { path = "crates/cronos-proto" }
cronos-common = { path = "crates/cronos-common" }
```

**Step 3: Run integration tests**

Run: `cargo test --test integration_test`
Expected: All 3 tests pass.

**Step 4: Commit**

```bash
git add Cargo.toml tests/
git commit -m "test: add integration tests for engine event handling, dedup, and handshake"
```

---

## Summary

| Task | Component | Description |
|------|-----------|-------------|
| 1 | cronos-model | Domain types: Entity, Event, Edge, IDs, enums |
| 2 | cronos-common | Config, XDG paths, errors, tracing |
| 3 | cronos-proto | IPC messages, length-prefixed framing |
| 4 | cronos-core/storage | SQLite schema, migrations, CRUD |
| 5 | cronos-core/graph | In-memory petgraph, rebuild from storage |
| 6 | cronos-core/ingest+linker | Event dedup, entity resolution, edge creation |
| 7 | cronos-core/server+engine | Unix socket server, message routing |
| 8 | cronos CLI | Single binary with subcommands |
| 9 | cronos-collect-fs | Filesystem collector (inotify) |
| 10 | systemd | Service template |
| 11 | Integration tests | End-to-end engine tests |

Tasks are ordered by dependency: each task only depends on crates built in previous tasks.
