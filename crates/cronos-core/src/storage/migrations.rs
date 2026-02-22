use rusqlite::Connection;

const CURRENT_VERSION: i32 = 2;

/// Run all migrations on the given database connection.
///
/// Sets pragmas for WAL journal mode and foreign keys, then creates
/// the schema tables if they do not already exist. This function is
/// idempotent: calling it multiple times on the same database is safe.
pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    // Set pragmas
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    // Create schema_version table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL
        );",
    )?;

    let version: Option<i32> = conn
        .query_row(
            "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();

    let current = version.unwrap_or(0);

    if current >= CURRENT_VERSION {
        return Ok(());
    }

    // === V1 migrations ===
    if current < 1 {

    // === entities table ===
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entities (
            id         TEXT PRIMARY KEY NOT NULL,
            kind       TEXT NOT NULL,
            name       TEXT NOT NULL,
            attributes TEXT NOT NULL DEFAULT '{}',
            first_seen INTEGER NOT NULL,
            last_seen  INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_entities_kind     ON entities(kind);
        CREATE INDEX IF NOT EXISTS idx_entities_name     ON entities(name);
        CREATE INDEX IF NOT EXISTS idx_entities_last_seen ON entities(last_seen);",
    )?;

    // === events table ===
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS events (
            id         TEXT PRIMARY KEY NOT NULL,
            timestamp  INTEGER NOT NULL,
            source     TEXT NOT NULL,
            kind       TEXT NOT NULL,
            subject_id TEXT NOT NULL REFERENCES entities(id),
            metadata   TEXT NOT NULL DEFAULT '{}'
        );

        CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_events_source    ON events(source);
        CREATE INDEX IF NOT EXISTS idx_events_subject   ON events(subject_id);",
    )?;

    // === event_context junction table ===
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS event_context (
            event_id  TEXT NOT NULL REFERENCES events(id),
            entity_id TEXT NOT NULL REFERENCES entities(id),
            PRIMARY KEY (event_id, entity_id)
        );",
    )?;

    // === edges table ===
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS edges (
            id              TEXT PRIMARY KEY NOT NULL,
            from_id         TEXT NOT NULL REFERENCES entities(id),
            to_id           TEXT NOT NULL REFERENCES entities(id),
            relation        TEXT NOT NULL,
            strength        REAL NOT NULL DEFAULT 1.0,
            created_at      INTEGER NOT NULL,
            last_reinforced INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_edges_from     ON edges(from_id);
        CREATE INDEX IF NOT EXISTS idx_edges_to       ON edges(to_id);
        CREATE INDEX IF NOT EXISTS idx_edges_relation ON edges(relation);",
    )?;

    // === FTS5 virtual table for entity search ===
    // FTS5 tables cannot use IF NOT EXISTS directly, so we check first
    let fts_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='entities_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if !fts_exists {
        conn.execute_batch(
            "CREATE VIRTUAL TABLE entities_fts USING fts5(name, attributes, content='entities', content_rowid='rowid');",
        )?;
    }

    } // end v1

    // === V2 migrations: sessions table ===
    if current < 2 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id            TEXT PRIMARY KEY NOT NULL,
                app_name      TEXT NOT NULL,
                window_titles TEXT NOT NULL DEFAULT '[]',
                project       TEXT,
                category      TEXT NOT NULL DEFAULT 'other',
                start_time    INTEGER NOT NULL,
                end_time      INTEGER NOT NULL,
                duration_secs INTEGER NOT NULL,
                event_count   INTEGER NOT NULL DEFAULT 0,
                metadata      TEXT NOT NULL DEFAULT '{}'
            );

            CREATE INDEX IF NOT EXISTS idx_sessions_time     ON sessions(start_time, end_time);
            CREATE INDEX IF NOT EXISTS idx_sessions_app      ON sessions(app_name);
            CREATE INDEX IF NOT EXISTS idx_sessions_category ON sessions(category);",
        )?;
    } // end v2

    // Record schema version
    if version.is_none() {
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [CURRENT_VERSION],
        )?;
    } else {
        conn.execute(
            "UPDATE schema_version SET version = ?1",
            [CURRENT_VERSION],
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_run_on_fresh_db() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Verify tables exist by querying them
        let count: i32 = conn
            .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i32 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i32 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        let count: i32 = conn
            .query_row("SELECT COUNT(*) FROM event_context", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);

        // Verify schema version was recorded
        let version: i32 = conn
            .query_row("SELECT version FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);

        // Verify WAL journal mode was set.
        // In-memory databases always report "memory" regardless of the pragma,
        // so we just check it is one of the expected values.
        let journal: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert!(
            journal == "wal" || journal == "memory",
            "unexpected journal_mode: {journal}"
        );

        // Verify foreign keys enabled
        let fk: i32 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        // Run again — should not error
        run_migrations(&conn).unwrap();

        // Still version 1
        let version: i32 = conn
            .query_row("SELECT version FROM schema_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);
    }
}
