use cronos_model::*;
use std::collections::HashMap;

pub struct IngestPipeline {
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
        if event.subject.identity.is_empty() {
            tracing::warn!(event_id = %event.id, "dropping event with empty subject identity");
            return None;
        }

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

    pub fn prune_cache(&mut self, before: Timestamp) {
        self.dedup_cache.retain(|_, ts| *ts >= before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_event(source: CollectorSource, identity: &str, timestamp: Timestamp) -> Event {
        Event {
            id: EventId::new(),
            timestamp,
            source,
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
        let event = make_event(CollectorSource::Filesystem, "/src/main.rs", 5000);
        let result = pipeline.process(event);
        assert!(result.is_some());
    }

    #[test]
    fn drops_empty_identity() {
        let mut pipeline = IngestPipeline::new(1000);
        let event = make_event(CollectorSource::Filesystem, "", 5000);
        let result = pipeline.process(event);
        assert!(result.is_none());
    }

    #[test]
    fn deduplicates_within_window() {
        let mut pipeline = IngestPipeline::new(1000);
        let e1 = make_event(CollectorSource::Filesystem, "/src/main.rs", 5000);
        let e2 = make_event(CollectorSource::Filesystem, "/src/main.rs", 5500);

        assert!(pipeline.process(e1).is_some());
        assert!(pipeline.process(e2).is_none());
    }

    #[test]
    fn allows_after_window() {
        let mut pipeline = IngestPipeline::new(1000);
        let e1 = make_event(CollectorSource::Filesystem, "/src/main.rs", 5000);
        let e2 = make_event(CollectorSource::Filesystem, "/src/main.rs", 6500);

        assert!(pipeline.process(e1).is_some());
        assert!(pipeline.process(e2).is_some());
    }

    #[test]
    fn different_files_not_deduped() {
        let mut pipeline = IngestPipeline::new(1000);
        let e1 = make_event(CollectorSource::Filesystem, "/src/main.rs", 5000);
        let e2 = make_event(CollectorSource::Filesystem, "/src/lib.rs", 5000);

        assert!(pipeline.process(e1).is_some());
        assert!(pipeline.process(e2).is_some());
    }
}
