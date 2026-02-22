use anyhow::{Context, Result};
use dialoguer::{theme::ColorfulTheme, Select};
use serde::{Deserialize, Serialize};
use std::path::Path;

const CONSENT_FILE: &str = "consent.json";

#[derive(Serialize, Deserialize)]
struct Consent {
    accepted: bool,
    timestamp: String,
}

fn separator() {
    println!("\n  │\n  │\n  │\n");
}

/// Check whether the user has already accepted the first-run disclaimer.
/// If not, display the banner + disclaimer and ask for explicit confirmation.
/// Writes `consent.json` on acceptance; exits the process on decline.
/// Returns `true` if this was a fresh first-run (user just accepted now).
pub fn check_consent(config_dir: &Path) -> Result<bool> {
    let consent_path = config_dir.join(CONSENT_FILE);
    if consent_path.exists() {
        return Ok(false);
    }

    print_banner();
    print_disclaimer();

    let theme = ColorfulTheme::default();
    let choices = &["Yes", "No"];

    // First confirmation
    let selection = Select::with_theme(&theme)
        .with_prompt("Do you understand that Cronos tracks your activity and sends data to an LLM provider?")
        .items(choices)
        .default(1)
        .interact()
        .context("reading user input")?;

    if selection != 0 {
        println!("\nYou must accept the terms to use Cronos. Exiting.");
        std::process::exit(0);
    }

    separator();

    // Second confirmation
    let selection = Select::with_theme(&theme)
        .with_prompt("Do you accept these terms and wish to proceed?")
        .items(choices)
        .default(1)
        .interact()
        .context("reading user input")?;

    if selection != 0 {
        println!("\nYou must accept the terms to use Cronos. Exiting.");
        std::process::exit(0);
    }

    // Write consent file
    let consent = Consent {
        accepted: true,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    let json = serde_json::to_string_pretty(&consent)
        .context("serializing consent")?;
    std::fs::write(&consent_path, json)
        .context("writing consent file")?;

    println!("\nConsent recorded. Welcome to Cronos!\n");
    Ok(true)
}

/// Prompt the user to connect their OpenAI account. Returns true if they want to.
pub fn prompt_openai_connect() -> Result<bool> {
    separator();

    let theme = ColorfulTheme::default();
    let choices = &["Yes", "No"];

    let selection = Select::with_theme(&theme)
        .with_prompt("Would you like to connect your OpenAI account now?")
        .items(choices)
        .default(0)
        .interact()
        .context("reading user input")?;

    Ok(selection == 0)
}

/// Prompt the user to install the Cronos desktop app. Returns true if they want to.
pub fn prompt_desktop_install() -> Result<bool> {
    separator();

    let theme = ColorfulTheme::default();
    let choices = &["Yes", "No"];

    let selection = Select::with_theme(&theme)
        .with_prompt("Would you like to install the Cronos desktop app? (this may take a few minutes)")
        .items(choices)
        .default(1)
        .interact()
        .context("reading user input")?;

    Ok(selection == 0)
}

fn print_banner() {
    const AMBER: &str = "\x1b[38;5;214m";
    const BOLD: &str = "\x1b[1m";
    const RESET: &str = "\x1b[0m";

    println!(
        r#"
{BOLD}{AMBER}
   ██████╗██████╗  ██████╗ ███╗   ██╗ ██████╗ ███████╗
  ██╔════╝██╔══██╗██╔═══██╗████╗  ██║██╔═══██╗██╔════╝
  ██║     ██████╔╝██║   ██║██╔██╗ ██║██║   ██║███████╗
  ██║     ██╔══██╗██║   ██║██║╚██╗██║██║   ██║╚════██║
  ╚██████╗██║  ██║╚██████╔╝██║ ╚████║╚██████╔╝███████║
   ╚═════╝╚═╝  ╚═╝ ╚═════╝ ╚═╝  ╚═══╝ ╚═════╝ ╚══════╝
{RESET}"#,
        BOLD = BOLD,
        AMBER = AMBER,
        RESET = RESET,
    );
}

fn print_disclaimer() {
    println!(
        r#"Cronos is an activity tracking tool for developers. Before you proceed,
please understand what it does:

  - Cronos monitors your active applications, window titles, and file
    changes in real time using background collector processes.

  - All collected data is stored locally in a SQLite database on your
    machine.

  - When you use the chat feature, your activity data is sent to an
    external LLM provider (OpenAI) to generate responses. The LLM
    provider may process, log, or retain this data according to their
    own privacy policies.

  - A poorly crafted prompt could cause the AI to expose or
    misinterpret your tracked activity data.

  - You are responsible for understanding the privacy implications
    of running this tool on your machine.
"#
    );
}
