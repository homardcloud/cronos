use crate::storage::repo::Session;
use crate::storage::Repository;
use cronos_model::{CollectorSource, EntityId, Timestamp};
use std::collections::HashMap;

/// A resolved event with the app name looked up from the entity table.
struct ResolvedEvent {
    timestamp: Timestamp,
    app_name: String,
    window_title: Option<String>,
}

/// Groups raw `AppFocused` events into meaningful activity sessions.
pub struct SessionAggregator {
    /// Maximum gap (in ms) between events to be considered part of the same session.
    session_gap_ms: u64,
}

impl SessionAggregator {
    pub fn new(session_gap_ms: u64) -> Self {
        Self { session_gap_ms }
    }

    /// Aggregate raw AppMonitor events since the last session watermark into sessions.
    ///
    /// Returns the number of sessions created.
    pub fn aggregate(&self, repo: &Repository) -> rusqlite::Result<usize> {
        let watermark = repo.last_session_end_time()?.unwrap_or(0);
        let now = cronos_common::now_ms();

        let all_events = repo.events_in_range(watermark, now)?;

        // Filter to AppMonitor source only, skip events at exact watermark
        let app_events: Vec<_> = all_events
            .iter()
            .filter(|e| e.source == CollectorSource::AppMonitor)
            .filter(|e| e.timestamp > watermark || watermark == 0)
            .collect();

        if app_events.is_empty() {
            return Ok(0);
        }

        // Resolve entity names (app names) via subject_id lookups, with caching
        let mut name_cache: HashMap<EntityId, String> = HashMap::new();
        let mut resolved: Vec<ResolvedEvent> = Vec::with_capacity(app_events.len());

        for event in &app_events {
            let app_name = if let Some(name) = name_cache.get(&event.subject_id) {
                name.clone()
            } else {
                let name = repo
                    .get_entity(event.subject_id)?
                    .map(|e| e.name)
                    .unwrap_or_else(|| "Unknown".to_string());
                name_cache.insert(event.subject_id, name.clone());
                name
            };

            let window_title = event
                .metadata
                .get("window_title")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            resolved.push(ResolvedEvent {
                timestamp: event.timestamp,
                app_name,
                window_title,
            });
        }

        let sessions = self.build_sessions(&resolved);
        let count = sessions.len();

        for session in &sessions {
            repo.insert_session(session)?;
        }

        Ok(count)
    }

    /// Build sessions from a sorted list of resolved events.
    fn build_sessions(&self, events: &[ResolvedEvent]) -> Vec<Session> {
        if events.is_empty() {
            return vec![];
        }

        let mut sessions = Vec::new();
        let mut current_app = events[0].app_name.clone();
        let mut current_start = events[0].timestamp;
        let mut current_end = events[0].timestamp;
        let mut current_titles: Vec<String> = Vec::new();
        let mut current_count: i64 = 0;

        if let Some(title) = &events[0].window_title {
            current_titles.push(title.clone());
        }
        current_count += 1;

        for event in events.iter().skip(1) {
            let gap = event.timestamp - current_end;

            if event.app_name == current_app && gap < self.session_gap_ms as i64 {
                // Extend current session
                current_end = event.timestamp;
                current_count += 1;
                if let Some(title) = &event.window_title {
                    if !current_titles.contains(title) {
                        current_titles.push(title.clone());
                    }
                }
            } else {
                // Finalize current session
                sessions.push(make_session(
                    &current_app,
                    &current_titles,
                    current_start,
                    current_end,
                    current_count,
                ));

                // Start new session
                current_app = event.app_name.clone();
                current_start = event.timestamp;
                current_end = event.timestamp;
                current_titles.clear();
                current_count = 1;
                if let Some(title) = &event.window_title {
                    current_titles.push(title.clone());
                }
            }
        }

        // Finalize last session
        sessions.push(make_session(
            &current_app,
            &current_titles,
            current_start,
            current_end,
            current_count,
        ));

        sessions
    }
}

fn make_session(
    app_name: &str,
    window_titles: &[String],
    start_time: Timestamp,
    end_time: Timestamp,
    event_count: i64,
) -> Session {
    let duration_secs = (end_time - start_time) / 1000;
    Session {
        id: ulid::Ulid::new().to_string(),
        app_name: app_name.to_string(),
        window_titles: window_titles.to_vec(),
        project: None,
        category: categorize_app(app_name).to_string(),
        start_time,
        end_time,
        duration_secs,
        event_count,
        metadata: HashMap::new(),
    }
}

/// Categorize an app by name into a broad activity category.
pub fn categorize_app(app_name: &str) -> &'static str {
    let lower = app_name.to_lowercase();
    if lower.contains("code")
        || lower.contains("xcode")
        || lower.contains("intellij")
        || lower.contains("terminal")
        || lower.contains("iterm")
        || lower.contains("warp")
        || lower.contains("alacritty")
        || lower.contains("kitty")
        || lower.contains("cursor")
    {
        "coding"
    } else if lower.contains("discord")
        || lower.contains("slack")
        || lower.contains("messages")
        || lower.contains("telegram")
        || lower.contains("teams")
        || lower.contains("mail")
    {
        "communication"
    } else if lower.contains("chrome")
        || lower.contains("firefox")
        || lower.contains("safari")
        || lower.contains("arc")
        || lower.contains("brave")
        || lower.contains("edge")
    {
        "browsing"
    } else if lower.contains("notion")
        || lower.contains("obsidian")
        || lower.contains("notes")
        || lower.contains("pages")
        || lower.contains("docs")
    {
        "productivity"
    } else if lower.contains("spotify")
        || lower.contains("music")
        || lower.contains("vlc")
    {
        "media"
    } else if lower.contains("finder") || lower.contains("preview") {
        "system"
    } else {
        "other"
    }
}

/// Spawn a background task that periodically aggregates events into sessions.
pub fn spawn_aggregator(
    engine: std::sync::Arc<crate::engine::Engine>,
    interval_secs: u64,
    session_gap_ms: u64,
) {
    let aggregator = SessionAggregator::new(session_gap_ms);
    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs(interval_secs);
        loop {
            tokio::time::sleep(interval).await;
            match engine.run_aggregator(&aggregator) {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(sessions = count, "aggregated new sessions");
                    }
                }
                Err(e) => {
                    tracing::warn!("aggregation error: {e}");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use cronos_model::*;
    use std::collections::HashMap;

    #[test]
    fn categorize_app_identifies_coding_apps() {
        assert_eq!(categorize_app("VS Code"), "coding");
        assert_eq!(categorize_app("Cursor"), "coding");
        assert_eq!(categorize_app("Terminal"), "coding");
        assert_eq!(categorize_app("iTerm2"), "coding");
        assert_eq!(categorize_app("Xcode"), "coding");
    }

    #[test]
    fn categorize_app_identifies_communication_apps() {
        assert_eq!(categorize_app("Discord"), "communication");
        assert_eq!(categorize_app("Slack"), "communication");
        assert_eq!(categorize_app("Messages"), "communication");
        assert_eq!(categorize_app("Microsoft Teams"), "communication");
    }

    #[test]
    fn categorize_app_identifies_browsing_apps() {
        assert_eq!(categorize_app("Google Chrome"), "browsing");
        assert_eq!(categorize_app("Arc"), "browsing");
        assert_eq!(categorize_app("Safari"), "browsing");
        assert_eq!(categorize_app("Firefox"), "browsing");
    }

    #[test]
    fn categorize_app_identifies_other_categories() {
        assert_eq!(categorize_app("Notion"), "productivity");
        assert_eq!(categorize_app("Spotify"), "media");
        assert_eq!(categorize_app("Finder"), "system");
        assert_eq!(categorize_app("SomeRandomApp"), "other");
    }

    #[test]
    fn build_sessions_groups_consecutive_same_app() {
        let aggregator = SessionAggregator::new(30_000);
        let events = vec![
            ResolvedEvent {
                timestamp: 1000,
                app_name: "VS Code".to_string(),
                window_title: Some("main.rs".to_string()),
            },
            ResolvedEvent {
                timestamp: 4000,
                app_name: "VS Code".to_string(),
                window_title: Some("lib.rs".to_string()),
            },
            ResolvedEvent {
                timestamp: 7000,
                app_name: "VS Code".to_string(),
                window_title: Some("main.rs".to_string()),
            },
        ];
        let sessions = aggregator.build_sessions(&events);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].app_name, "VS Code");
        assert_eq!(sessions[0].start_time, 1000);
        assert_eq!(sessions[0].end_time, 7000);
        assert_eq!(sessions[0].event_count, 3);
        assert_eq!(sessions[0].category, "coding");
        assert!(sessions[0].window_titles.contains(&"main.rs".to_string()));
        assert!(sessions[0].window_titles.contains(&"lib.rs".to_string()));
    }

    #[test]
    fn build_sessions_splits_on_app_change() {
        let aggregator = SessionAggregator::new(30_000);
        let events = vec![
            ResolvedEvent {
                timestamp: 1000,
                app_name: "VS Code".to_string(),
                window_title: Some("main.rs".to_string()),
            },
            ResolvedEvent {
                timestamp: 4000,
                app_name: "Discord".to_string(),
                window_title: Some("#general".to_string()),
            },
            ResolvedEvent {
                timestamp: 7000,
                app_name: "Discord".to_string(),
                window_title: Some("#random".to_string()),
            },
        ];
        let sessions = aggregator.build_sessions(&events);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].app_name, "VS Code");
        assert_eq!(sessions[0].category, "coding");
        assert_eq!(sessions[1].app_name, "Discord");
        assert_eq!(sessions[1].category, "communication");
        assert_eq!(sessions[1].event_count, 2);
    }

    #[test]
    fn build_sessions_splits_on_large_gap() {
        let aggregator = SessionAggregator::new(30_000); // 30s gap
        let events = vec![
            ResolvedEvent {
                timestamp: 1000,
                app_name: "VS Code".to_string(),
                window_title: None,
            },
            ResolvedEvent {
                timestamp: 50_000, // 49s gap, exceeds 30s threshold
                app_name: "VS Code".to_string(),
                window_title: None,
            },
        ];
        let sessions = aggregator.build_sessions(&events);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].app_name, "VS Code");
        assert_eq!(sessions[1].app_name, "VS Code");
    }

    #[test]
    fn build_sessions_deduplicates_window_titles() {
        let aggregator = SessionAggregator::new(30_000);
        let events = vec![
            ResolvedEvent {
                timestamp: 1000,
                app_name: "VS Code".to_string(),
                window_title: Some("main.rs".to_string()),
            },
            ResolvedEvent {
                timestamp: 4000,
                app_name: "VS Code".to_string(),
                window_title: Some("main.rs".to_string()),
            },
        ];
        let sessions = aggregator.build_sessions(&events);
        assert_eq!(sessions[0].window_titles.len(), 1);
    }

    #[test]
    fn aggregate_with_real_repo() {
        let repo = Repository::open_in_memory().unwrap();

        // Create an app entity
        let entity = Entity {
            id: EntityId::new(),
            kind: EntityKind::App,
            name: "VS Code".to_string(),
            attributes: HashMap::new(),
            first_seen: 1000,
            last_seen: 7000,
        };
        repo.insert_entity(&entity).unwrap();

        // Insert AppMonitor events
        for ts in [1000, 4000, 7000] {
            let mut metadata = HashMap::new();
            metadata.insert(
                "window_title".to_string(),
                serde_json::json!("main.rs"),
            );
            let event = Event {
                id: EventId::new(),
                timestamp: ts,
                source: CollectorSource::AppMonitor,
                kind: EventKind::AppFocused,
                subject: EntityRef {
                    kind: EntityKind::App,
                    identity: "VS Code".to_string(),
                    attributes: HashMap::new(),
                },
                context: vec![],
                metadata,
            };
            repo.insert_event(&event, entity.id, &[]).unwrap();
        }

        let aggregator = SessionAggregator::new(30_000);
        let count = aggregator.aggregate(&repo).unwrap();
        assert_eq!(count, 1);

        let sessions = repo.sessions_in_range(0, 100_000, 50).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].app_name, "VS Code");
        assert_eq!(sessions[0].event_count, 3);

        // Running again should produce 0 new sessions (watermark prevents reprocessing)
        let count2 = aggregator.aggregate(&repo).unwrap();
        assert_eq!(count2, 0);
    }
}
