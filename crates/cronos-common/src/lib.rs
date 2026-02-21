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
