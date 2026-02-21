use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
        "**/node_modules/**".to_string(),
        "**/.git/objects/**".to_string(),
        "**/target/**".to_string(),
        "**/.cache/**".to_string(),
        "**/*.swp".to_string(),
        "**/*.tmp".to_string(),
    ]
}
fn default_debounce() -> u64 { 500 }
fn default_browser_port() -> u16 { 19280 }
fn default_dwell_time() -> u64 { 3000 }

// Default impls for config structs with non-trivial defaults
impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: String::new(), db_path: String::new(),
            log_level: default_log_level(), event_channel_size: default_channel_size(),
            dedup: DedupConfig::default(), linker: LinkerConfig::default(),
        }
    }
}
impl Default for DedupConfig {
    fn default() -> Self { Self { window_ms: default_dedup_window() } }
}
impl Default for LinkerConfig {
    fn default() -> Self {
        Self { temporal_window_ms: default_temporal_window(), min_edge_strength: default_min_edge_strength() }
    }
}
impl Default for FsCollectorConfig {
    fn default() -> Self {
        Self { enabled: default_true(), watch_paths: default_watch_paths(), ignore_patterns: default_ignore_patterns(), debounce_ms: default_debounce() }
    }
}
impl Default for BrowserCollectorConfig {
    fn default() -> Self {
        Self { enabled: false, listen_port: default_browser_port(), ignore_domains: Vec::new(), min_dwell_time_ms: default_dwell_time() }
    }
}

impl CronosConfig {
    pub fn load(path: &Path) -> crate::error::Result<Self> {
        let mut config = Self::default();
        if path.exists() {
            let content = std::fs::read_to_string(path)
                .map_err(|e| crate::error::CronosError::Config(format!("read config: {}", e)))?;
            config = toml::from_str(&content)
                .map_err(|e| crate::error::CronosError::Config(format!("parse config: {}", e)))?;
        }
        if let Ok(v) = std::env::var("CRONOS_LOG_LEVEL") { config.daemon.log_level = v; }
        if let Ok(v) = std::env::var("CRONOS_SOCKET_PATH") { config.daemon.socket_path = v; }
        if let Ok(v) = std::env::var("CRONOS_DB_PATH") { config.daemon.db_path = v; }
        Ok(config)
    }
}

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
