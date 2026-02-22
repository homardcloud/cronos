use std::io::{self, BufRead, Write};

use crate::CronosPaths;

const CONSENT_FILE: &str = "consent.json";

/// Check if this is the first run (consent.json doesn't exist).
pub fn is_first_run() -> anyhow::Result<bool> {
    let paths = CronosPaths::resolve()?;
    Ok(!paths.config_dir.join(CONSENT_FILE).exists())
}

/// Mark setup as complete by writing consent.json.
pub fn mark_setup_done() -> anyhow::Result<()> {
    let paths = CronosPaths::resolve()?;
    std::fs::create_dir_all(&paths.config_dir)?;
    let data = serde_json::json!({
        "setup_complete": true,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(
        paths.config_dir.join(CONSENT_FILE),
        serde_json::to_string_pretty(&data)?,
    )?;
    Ok(())
}

/// Prompt the user to connect their OpenAI/ChatGPT account.
/// Returns true if the user wants to connect.
pub fn prompt_openai_connect() -> anyhow::Result<bool> {
    println!();
    println!("────────────────────────────────────────────────────");
    println!();
    println!("  Cronos can connect to OpenAI to power an AI chat");
    println!("  assistant that knows about your daily activity.");
    println!();
    prompt_yes_no("Would you like to connect your OpenAI account?", true)
}

/// Prompt the user to install the Cronos desktop app.
/// Returns true if the user wants to install.
pub fn prompt_desktop_install() -> anyhow::Result<bool> {
    println!();
    println!("────────────────────────────────────────────────────");
    println!();
    println!("  Cronos also has a desktop app with a visual");
    println!("  dashboard for viewing your activity timeline,");
    println!("  session breakdowns, and chatting with the AI");
    println!("  assistant.");
    println!();
    prompt_yes_no(
        "Would you like to install the desktop app? (this may take a few minutes)",
        false,
    )
}

/// Simple yes/no prompt. Returns Ok(true) for yes, Ok(false) for no.
fn prompt_yes_no(question: &str, default_yes: bool) -> anyhow::Result<bool> {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("  {question} {hint} ");
    io::stdout().flush()?;

    let stdin = io::stdin();
    let line = stdin.lock().lines().next().unwrap_or(Ok(String::new()))?;
    let answer = line.trim().to_lowercase();

    if answer.is_empty() {
        return Ok(default_yes);
    }

    Ok(answer.starts_with('y'))
}
