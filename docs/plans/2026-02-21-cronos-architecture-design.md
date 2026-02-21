# Cronos Architecture Design

## What Cronos Is

A local-first context engine that runs as a daemon on the user's machine and builds a structured "context graph" over time based on opt-in activity observation. It does not scan the disk or ingest retroactively — it builds understanding as the user works, linking activity across sources (files, browser, git, terminal, Discord, etc.) into a temporal knowledge graph.

## Decisions

- **Language:** Rust
- **Platform:** Linux first, macOS and Windows later (design with platform abstraction)
- **Architecture:** Plugin architecture — core daemon + separate collector binaries over Unix socket
- **Storage:** SQLite (WAL mode) + in-memory graph (petgraph)
- **Protocol:** Length-prefixed JSON over Unix socket, versioned
- **Config:** TOML, XDG-compliant paths
- **CLI:** Single `cronos` binary with subcommands
- **Daemon:** Foreground dev mode + systemd user service

## Project Structure

```
cronos/
├── Cargo.toml                        # workspace root
├── crates/
│   ├── cronos/                       # single binary entry point
│   │   └── src/main.rs               # subcommands: daemon, install, status, query, timeline, collectors
│   │
│   ├── cronos-core/                  # daemon engine (lib crate)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── graph/                # in-memory context graph (petgraph)
│   │       ├── storage/              # SQLite persistence (rusqlite)
│   │       ├── server/               # Unix socket server (event ingestion + query)
│   │       ├── linker/               # entity resolution, dedup, cross-source linking
│   │       └── ingest/               # event normalization pipeline
│   │
│   ├── cronos-model/                 # domain model only
│   │   └── src/lib.rs                # Event, Node, Edge, EntityId, Timestamp, enums
│   │
│   ├── cronos-proto/                 # IPC protocol
│   │   └── src/lib.rs                # Request/Response messages, framing, versioning, serde
│   │
│   ├── cronos-common/                # shared utilities
│   │   └── src/lib.rs                # config, tracing, errors, ID gen, socket client, XDG paths
│   │
│   └── collectors/
│       ├── cronos-collect-fs/        # inotify file watcher
│       └── cronos-collect-browser/   # browser extension bridge (local HTTP → Unix socket)
│
├── assets/
│   └── systemd/
│       └── cronos.service            # systemd user service template
│
├── config/
│   └── cronos.default.toml
│
└── docs/
    └── plans/
```

## Data Model (`cronos-model`)

### Entity

Things Cronos knows about, discovered from events.

```rust
struct Entity {
    id: EntityId,                   // ULID
    kind: EntityKind,               // Project, File, Repo, Url, App, ...
    name: String,
    attributes: Map<String, Value>,
    first_seen: Timestamp,
    last_seen: Timestamp,
}

enum EntityKind {
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
```

### Event

Raw observations from collectors. Immutable, append-only.

```rust
struct Event {
    id: EventId,                    // ULID
    timestamp: Timestamp,
    source: CollectorSource,
    kind: EventKind,                // FileOpened, FileModified, UrlVisited, ...
    subject: EntityRef,             // primary entity
    context: Vec<EntityRef>,        // related entities
    metadata: Map<String, Value>,
}

struct EntityRef {
    kind: EntityKind,
    identity: String,               // path, url, hash — unique identifier
    attributes: Map<String, Value>,
}
```

### Edge

Relationships in the graph, created by the linker (not collectors).

```rust
struct Edge {
    id: EdgeId,                     // ULID
    from: EntityId,
    to: EntityId,
    relation: Relation,
    strength: f32,                  // 0.0-1.0, reinforced over time
    created_at: Timestamp,
    last_reinforced: Timestamp,
}

enum Relation {
    BelongsTo,
    Contains,
    References,
    OccurredDuring,
    Visited,
    RelatedTo,
    Custom(String),
}
```

### Data Flow

1. Collector emits `Event` with `EntityRef`s (dumb, raw)
2. `cronos-core/ingest` normalizes and validates
3. `cronos-core/linker` resolves `EntityRef`s to `Entity` (find or create)
4. Linker creates/reinforces `Edge`s based on co-occurrence and context
5. Graph + SQLite updated

## IPC Protocol (`cronos-proto`)

### Wire Format

Length-prefixed JSON over Unix socket.

```
┌──────────────┬─────────────────────┐
│ length: u32  │ payload: JSON bytes │
│ (4 bytes LE) │ (length bytes)      │
└──────────────┴─────────────────────┘
```

### Messages

```rust
struct Message {
    version: u8,
    id: String,
    kind: MessageKind,
}

enum MessageKind {
    // Collector -> Core
    EmitEvent(Event),
    CollectorHandshake { name: String, version: String, source: CollectorSource },
    Heartbeat,

    // CLI -> Core
    Query(QueryRequest),
    Status,
    ListCollectors,

    // Core -> Collector/CLI
    Ack { request_id: String },
    Error { request_id: String, code: ErrorCode, message: String },
    QueryResult(QueryResponse),
    StatusResult(StatusInfo),
}
```

### Query Protocol

```rust
enum QueryKind {
    Search { text: String, limit: u32 },
    Timeline { from: Timestamp, to: Timestamp },
    Related { entity_id: EntityId, depth: u8 },
    Recent { limit: u32 },
}

struct QueryResponse {
    entities: Vec<Entity>,
    edges: Vec<Edge>,
    events: Vec<Event>,
}
```

### Collector Lifecycle

1. Connect to `$XDG_RUNTIME_DIR/cronos/cronos.sock`
2. Send `CollectorHandshake` → core responds `Ack`
3. Send `EmitEvent` messages → core responds `Ack` per event
4. Send `Heartbeat` every 30s
5. On disconnect: reconnect with backoff

## Core Daemon Architecture

```
                    Unix Socket Server
                    ┌─────────────────┐
                    │  accept loop    │
                    │  (tokio tasks)  │
                    └────────┬────────┘
                             │
              ┌──────────────┼──────────────┐
              ▼              ▼              ▼
        collector         collector       CLI
        connection        connection      connection
              │              │              │
              ▼              ▼              │
        ┌─────────────────────────┐        │
        │     Event Channel       │        │
        │  (tokio mpsc, bounded)  │        │
        └────────────┬────────────┘        │
                     ▼                     │
        ┌─────────────────────────┐        │
        │     Ingest Pipeline     │        │
        │  - validate event       │        │
        │  - normalize timestamps │        │
        │  - deduplicate          │        │
        └────────────┬────────────┘        │
                     ▼                     │
        ┌─────────────────────────┐        │
        │       Linker            │        │
        │  - resolve EntityRefs   │        │
        │  - find/create Entities │        │
        │  - create/reinforce     │        │
        │    Edges                │        │
        └──────┬──────────┬───────┘        │
               ▼          ▼                ▼
        ┌───────────┐ ┌────────────┐ ┌───────────┐
        │  In-Memory│ │   SQLite   │ │  Query    │
        │  Graph    │ │  Storage   │ │  Engine   │
        │ (petgraph)│ │ (rusqlite) │ │           │
        └───────────┘ └────────────┘ └───────────┘
```

- **Socket server:** tokio task per connection, routes messages
- **Event channel:** bounded mpsc with backpressure
- **Ingest:** validation, timestamp normalization, dedup (same source + subject within time window)
- **Linker:** resolves EntityRefs to Entities, creates edges based on co-occurrence, temporal proximity, containment
- **In-memory graph:** petgraph, rebuilt from SQLite on startup
- **SQLite:** WAL mode, concurrent reads during writes
- **Query engine:** graph for traversals, SQLite for full-text search and time ranges

### Startup Sequence

1. Load config from `$XDG_CONFIG_HOME/cronos/config.toml`
2. Open/create SQLite DB at `$XDG_DATA_HOME/cronos/cronos.db`
3. Run migrations if needed
4. Rebuild in-memory graph from SQLite
5. Bind Unix socket at `$XDG_RUNTIME_DIR/cronos/cronos.sock`
6. Start accept loop

### Shutdown

SIGTERM/SIGINT → stop accepting, drain event channel, flush SQLite WAL, exit.

## Collectors

### `cronos-collect-fs`

Watches configured directories via inotify. Emits: `FileOpened`, `FileModified`, `FileCreated`, `FileDeleted`. Detects project roots by walking up to find `.git/`, `Cargo.toml`, `package.json`, etc. Includes project as context EntityRef.

### `cronos-collect-browser`

Tiny HTTP server receiving POSTs from a browser extension. Emits: `UrlVisited`, `TabFocused`. Filters by min dwell time and ignored domains. The browser extension is a minimal WebExtension (~50 lines JS) that fires on tab focus/navigation.

### Adding New Collectors

1. Create crate under `collectors/`
2. Use `cronos-common` for socket + config
3. Define EventKinds
4. Ship as separate binary — no core changes needed

## Configuration

File: `$XDG_CONFIG_HOME/cronos/config.toml`

Resolution order: built-in defaults → config file → env vars (`CRONOS_*`) → CLI flags.

```toml
[daemon]
socket_path = ""                    # default: $XDG_RUNTIME_DIR/cronos/cronos.sock
db_path = ""                        # default: $XDG_DATA_HOME/cronos/cronos.db
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
ignore_patterns = ["*/node_modules/*", "*/.git/objects/*", "*/target/*", "*/.cache/*", "*.swp", "*.tmp"]
debounce_ms = 500

[collectors.browser]
enabled = false
listen_port = 19280
ignore_domains = []
min_dwell_time_ms = 3000
```

## CLI Subcommands

```
cronos daemon                       # foreground
cronos daemon --detach              # background
cronos install                      # install systemd user service
cronos uninstall                    # remove systemd service
cronos init                         # generate default config
cronos status                       # daemon status, collectors, db stats
cronos query "search text"          # full-text search
cronos query --related <id>         # graph traversal
cronos query --related <id> --depth 3
cronos timeline                     # today
cronos timeline --from 2026-02-20 --to 2026-02-21
cronos timeline --last 2h
cronos recent                       # last 20 events
cronos recent --limit 50
cronos collectors                   # list collectors + status
cronos collectors restart fs        # restart collector
cronos config show                  # print resolved config
cronos config edit                  # open in $EDITOR
```

All query commands support `--json` for machine-readable output.

## SQLite Schema

```sql
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

CREATE TABLE schema_version (
    version     INTEGER NOT NULL
);
```
