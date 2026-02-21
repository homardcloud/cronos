use anyhow::{Context, Result};
use std::path::Path;

const AUTH_FILE: &str = "auth.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredCredentials {
    api_key: String,
}

pub fn save_api_key(config_dir: &Path, api_key: &str) -> Result<()> {
    let creds = StoredCredentials {
        api_key: api_key.to_string(),
    };
    let json = serde_json::to_string_pretty(&creds).context("serializing credentials")?;
    let path = config_dir.join(AUTH_FILE);
    std::fs::write(&path, &json).with_context(|| format!("writing {}", path.display()))?;

    // Set restrictive permissions (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms)
            .with_context(|| format!("setting permissions on {}", path.display()))?;
    }

    Ok(())
}

pub fn load_api_key(config_dir: &Path) -> Result<Option<String>> {
    let path = config_dir.join(AUTH_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let creds: StoredCredentials =
        serde_json::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(creds.api_key))
}

pub fn remove_credentials(config_dir: &Path) -> Result<()> {
    let path = config_dir.join(AUTH_FILE);
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}
