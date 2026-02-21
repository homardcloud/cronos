use directories::ProjectDirs;
use std::path::PathBuf;

const QUALIFIER: &str = "";
const ORG: &str = "";
const APP: &str = "cronos";

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
