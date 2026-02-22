use anyhow::{Context, Result};
use std::path::Path;

const AUTH_FILE: &str = "auth.json";

#[derive(serde::Serialize, serde::Deserialize)]
pub struct StoredCredentials {
    /// API key from platform org exchange (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// OAuth access_token for direct API use (ChatGPT Plus/Pro).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    /// OAuth refresh_token for renewing the access_token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

pub fn save(config_dir: &Path, creds: &StoredCredentials) -> Result<()> {
    let json = serde_json::to_string_pretty(creds).context("serializing credentials")?;
    let path = config_dir.join(AUTH_FILE);
    std::fs::write(&path, &json).with_context(|| format!("writing {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms)
            .with_context(|| format!("setting permissions on {}", path.display()))?;
    }

    Ok(())
}

pub fn load(config_dir: &Path) -> Result<Option<StoredCredentials>> {
    let path = config_dir.join(AUTH_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let creds: StoredCredentials =
        serde_json::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(creds))
}

/// Return the best available API key/token from stored credentials.
pub fn load_api_key(config_dir: &Path) -> Result<Option<String>> {
    match load(config_dir)? {
        Some(creds) => {
            // Prefer api_key, fall back to access_token
            if let Some(ref key) = creds.api_key {
                if !key.is_empty() {
                    return Ok(Some(key.clone()));
                }
            }
            if let Some(ref token) = creds.access_token {
                if !token.is_empty() {
                    return Ok(Some(token.clone()));
                }
            }
            Ok(None)
        }
        None => Ok(None),
    }
}

pub fn remove_credentials(config_dir: &Path) -> Result<()> {
    let path = config_dir.join(AUTH_FILE);
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("removing {}", path.display()))?;
    }
    Ok(())
}
