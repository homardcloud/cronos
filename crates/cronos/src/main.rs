mod chat;

use anyhow::{anyhow, Context, Result};
use chrono::{NaiveDate, TimeZone, Utc};
use clap::{Parser, Subcommand};
use cronos_common::{CronosConfig, CronosPaths};
use cronos_core::engine::Engine;
use cronos_model::*;
use cronos_proto::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::UnixStream;

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "cronos", about = "Developer activity tracker", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the daemon in the foreground
    Daemon,

    /// Generate default configuration file
    Init,

    /// Show daemon status
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Search or traverse the context graph
    Query {
        /// Full-text search query
        text: Option<String>,

        /// Find entities related to the given entity ID
        #[arg(long)]
        related: Option<String>,

        /// Traversal depth for --related
        #[arg(long, default_value = "2")]
        depth: u8,

        /// Maximum number of results
        #[arg(long, default_value = "20")]
        limit: u32,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show recent events
    Recent {
        /// Maximum number of results
        #[arg(long, default_value = "20")]
        limit: u32,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show events in a time range
    Timeline {
        /// Start date (YYYY-MM-DD or RFC 3339)
        #[arg(long)]
        from: Option<String>,

        /// End date (YYYY-MM-DD or RFC 3339)
        #[arg(long)]
        to: Option<String>,

        /// Duration to look back (e.g. "2h", "30m", "60s")
        #[arg(long)]
        last: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Start an interactive AI chat session
    Chat {
        /// OpenAI model to use (overrides config)
        #[arg(long)]
        model: Option<String>,
    },

    /// Log in with your ChatGPT account (OAuth)
    Login,

    /// Log out and remove stored credentials
    Logout,

    /// Configuration subcommands
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print the resolved configuration
    Show,
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon => cmd_daemon().await,
        Commands::Init => cmd_init(),
        Commands::Status { json } => cmd_status(json).await,
        Commands::Query {
            text,
            related,
            depth,
            limit,
            json,
        } => cmd_query(text, related, depth, limit, json).await,
        Commands::Recent { limit, json } => cmd_recent(limit, json).await,
        Commands::Timeline {
            from,
            to,
            last,
            json,
        } => cmd_timeline(from, to, last, json).await,
        Commands::Chat { model } => chat::cmd_chat(model).await,
        Commands::Login => chat::cmd_login().await,
        Commands::Logout => chat::cmd_logout().await,
        Commands::Config { action } => match action {
            ConfigAction::Show => cmd_config_show(),
        },
    }
}

// ---------------------------------------------------------------------------
// Daemon command
// ---------------------------------------------------------------------------

async fn cmd_daemon() -> Result<()> {
    let paths = CronosPaths::resolve().context("resolving paths")?;
    let config = CronosConfig::load(&paths.config_file).context("loading config")?;

    cronos_common::init_tracing(&config.daemon.log_level);

    let db_path = resolve_db_path(&config, &paths);
    let socket_path = resolve_socket_path(&config, &paths);

    tracing::info!(db = %db_path.display(), socket = %socket_path.display(), "starting daemon");

    let engine = Engine::open(&db_path, &config.daemon).context("opening engine")?;
    let engine = Arc::new(engine);

    let engine_ref = Arc::clone(&engine);
    let server_handle = tokio::spawn(async move {
        cronos_core::server::run(engine_ref, &socket_path).await
    });

    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    server_handle.abort();
    let _ = server_handle.await;

    Ok(())
}

// ---------------------------------------------------------------------------
// Init command
// ---------------------------------------------------------------------------

fn cmd_init() -> Result<()> {
    let paths = CronosPaths::resolve().context("resolving paths")?;

    std::fs::create_dir_all(&paths.config_dir)
        .context("creating config directory")?;

    if paths.config_file.exists() {
        println!("Config file already exists: {}", paths.config_file.display());
        return Ok(());
    }

    let default_config = include_str!("../../../config/cronos.default.toml");
    std::fs::write(&paths.config_file, default_config)
        .context("writing config file")?;

    println!("Created config: {}", paths.config_file.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Status command
// ---------------------------------------------------------------------------

async fn cmd_status(json: bool) -> Result<()> {
    let msg = Message::new(new_request_id(), MessageKind::Status);
    let response = send_request(msg).await?;
    print_response(&response, json);
    Ok(())
}

// ---------------------------------------------------------------------------
// Query command
// ---------------------------------------------------------------------------

async fn cmd_query(
    text: Option<String>,
    related: Option<String>,
    depth: u8,
    limit: u32,
    json: bool,
) -> Result<()> {
    let query_kind = if let Some(entity_id_str) = related {
        let ulid = ulid::Ulid::from_string(&entity_id_str)
            .map_err(|e| anyhow!("invalid entity ID '{}': {}", entity_id_str, e))?;
        QueryKind::Related {
            entity_id: EntityId(ulid),
            depth,
        }
    } else if let Some(text) = text {
        QueryKind::Search { text, limit }
    } else {
        return Err(anyhow!(
            "provide a search query or --related <id>"
        ));
    };

    let msg = Message::new(
        new_request_id(),
        MessageKind::Query {
            query: QueryRequest { kind: query_kind },
        },
    );
    let response = send_request(msg).await?;
    print_response(&response, json);
    Ok(())
}

// ---------------------------------------------------------------------------
// Recent command
// ---------------------------------------------------------------------------

async fn cmd_recent(limit: u32, json: bool) -> Result<()> {
    let msg = Message::new(
        new_request_id(),
        MessageKind::Query {
            query: QueryRequest {
                kind: QueryKind::Recent { limit },
            },
        },
    );
    let response = send_request(msg).await?;
    print_response(&response, json);
    Ok(())
}

// ---------------------------------------------------------------------------
// Timeline command
// ---------------------------------------------------------------------------

async fn cmd_timeline(
    from: Option<String>,
    to: Option<String>,
    last: Option<String>,
    json: bool,
) -> Result<()> {
    let (from_ts, to_ts) = if let Some(duration_str) = last {
        let duration_ms = parse_duration_ms(&duration_str)?;
        let now = cronos_common::now_ms();
        (now - duration_ms, now)
    } else {
        let from_ts = from
            .as_deref()
            .map(parse_timestamp)
            .transpose()?
            .unwrap_or(0);
        let to_ts = to
            .as_deref()
            .map(parse_timestamp)
            .transpose()?
            .unwrap_or_else(cronos_common::now_ms);
        (from_ts, to_ts)
    };

    let msg = Message::new(
        new_request_id(),
        MessageKind::Query {
            query: QueryRequest {
                kind: QueryKind::Timeline {
                    from: from_ts,
                    to: to_ts,
                },
            },
        },
    );
    let response = send_request(msg).await?;
    print_response(&response, json);
    Ok(())
}

// ---------------------------------------------------------------------------
// Config show command
// ---------------------------------------------------------------------------

fn cmd_config_show() -> Result<()> {
    let paths = CronosPaths::resolve().context("resolving paths")?;
    let config = CronosConfig::load(&paths.config_file).context("loading config")?;
    let output = toml::to_string_pretty(&config).context("serializing config")?;
    println!("{}", output);
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers: IPC client
// ---------------------------------------------------------------------------

async fn connect() -> Result<UnixStream> {
    let paths = CronosPaths::resolve().context("resolving paths")?;
    let config = CronosConfig::load(&paths.config_file).context("loading config")?;
    let socket_path = resolve_socket_path(&config, &paths);

    UnixStream::connect(&socket_path)
        .await
        .with_context(|| format!(
            "cannot connect to daemon at {}. Is it running?",
            socket_path.display()
        ))
}

async fn send_request(msg: Message) -> Result<Message> {
    let stream = connect().await?;
    let (mut reader, mut writer) = stream.into_split();
    write_frame(&mut writer, &msg).await.context("sending request")?;
    let response = read_frame(&mut reader).await.context("reading response")?;
    Ok(response)
}

fn new_request_id() -> String {
    ulid::Ulid::new().to_string()
}

// ---------------------------------------------------------------------------
// Helpers: path resolution
// ---------------------------------------------------------------------------

fn resolve_db_path(config: &CronosConfig, paths: &CronosPaths) -> PathBuf {
    if config.daemon.db_path.is_empty() {
        paths.db_file.clone()
    } else {
        PathBuf::from(&config.daemon.db_path)
    }
}

fn resolve_socket_path(config: &CronosConfig, paths: &CronosPaths) -> PathBuf {
    if config.daemon.socket_path.is_empty() {
        paths.socket_file.clone()
    } else {
        PathBuf::from(&config.daemon.socket_path)
    }
}

// ---------------------------------------------------------------------------
// Helpers: parsing
// ---------------------------------------------------------------------------

/// Parse a human-friendly duration string like "2h", "30m", "60s" into
/// milliseconds.
fn parse_duration_ms(s: &str) -> Result<i64> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow!("empty duration string"));
    }

    let (num_part, unit) = s.split_at(s.len() - 1);
    let value: i64 = num_part
        .parse()
        .with_context(|| format!("invalid number in duration '{}'", s))?;

    let multiplier: i64 = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return Err(anyhow!("unknown duration unit '{}' (use s, m, h, d)", unit)),
    };

    Ok(value * multiplier)
}

/// Parse a timestamp string. Accepts YYYY-MM-DD or RFC 3339 format.
fn parse_timestamp(s: &str) -> Result<i64> {
    // Try RFC 3339 first (e.g. "2026-02-21T10:00:00Z")
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp_millis());
    }

    // Try YYYY-MM-DD
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let dt = Utc
            .from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap());
        return Ok(dt.timestamp_millis());
    }

    Err(anyhow!(
        "cannot parse '{}' as a date (expected YYYY-MM-DD or RFC 3339)",
        s
    ))
}

// ---------------------------------------------------------------------------
// Helpers: output formatting
// ---------------------------------------------------------------------------

fn print_response(msg: &Message, json: bool) {
    if json {
        match serde_json::to_string_pretty(msg) {
            Ok(s) => println!("{}", s),
            Err(e) => eprintln!("error serializing response: {}", e),
        }
        return;
    }

    match &msg.kind {
        MessageKind::StatusResult { info } => {
            println!("Cronos Daemon Status");
            println!("  Uptime:      {}",  format_uptime(info.uptime_secs));
            println!("  Entities:    {}",  info.entity_count);
            println!("  Edges:       {}",  info.edge_count);
            println!("  Events:      {}",  info.event_count);
            println!("  Collectors:  {}",  info.connected_collectors);
        }
        MessageKind::QueryResult { response } => {
            print_query_response(response);
        }
        MessageKind::Ack { request_id } => {
            println!("OK ({})", request_id);
        }
        MessageKind::Error {
            code, message, ..
        } => {
            eprintln!("Error [{:?}]: {}", code, message);
        }
        MessageKind::CollectorList { collectors } => {
            if collectors.is_empty() {
                println!("No collectors connected.");
            } else {
                for c in collectors {
                    println!(
                        "  {} ({:?}) - events: {}, connected: {}",
                        c.name, c.source, c.events_sent, c.connected
                    );
                }
            }
        }
        other => {
            println!("{:?}", other);
        }
    }
}

fn print_query_response(response: &QueryResponse) {
    if response.entities.is_empty() && response.events.is_empty() && response.edges.is_empty() {
        println!("No results.");
        return;
    }

    if !response.entities.is_empty() {
        println!("Entities ({}):", response.entities.len());
        for entity in &response.entities {
            println!(
                "  [{}] {} - {} (seen: {} .. {})",
                entity.kind,
                entity.id,
                entity.name,
                format_timestamp(entity.first_seen),
                format_timestamp(entity.last_seen),
            );
        }
    }

    if !response.events.is_empty() {
        println!("Events ({}):", response.events.len());
        for event in &response.events {
            println!(
                "  {} {:?}/{:?} on {} ({})",
                format_timestamp(event.timestamp),
                event.source,
                event.kind,
                event.subject.identity,
                event.id,
            );
        }
    }

    if !response.edges.is_empty() {
        println!("Edges ({}):", response.edges.len());
        for edge in &response.edges {
            println!(
                "  {} -> {} [{:?}] strength={:.2}",
                edge.from, edge.to, edge.relation, edge.strength,
            );
        }
    }
}

fn format_uptime(secs: u64) -> String {
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    if hours > 0 {
        format!("{}h {}m {}s", hours, mins, s)
    } else if mins > 0 {
        format!("{}m {}s", mins, s)
    } else {
        format!("{}s", s)
    }
}

fn format_timestamp(ms: Timestamp) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| format!("{}ms", ms))
}
