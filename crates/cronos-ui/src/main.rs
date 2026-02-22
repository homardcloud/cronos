#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cronos_chat::openai::{Auth, ChatMessage};
use cronos_chat::{autostart, credentials, oauth, openai, tools};
use cronos_common::{CronosConfig, CronosPaths};
use cronos_proto::{Message, MessageKind};
use std::path::PathBuf;
use std::sync::Mutex;

struct AppState {
    auth: Mutex<Option<Auth>>,
    socket_path: PathBuf,
    model: String,
    config_dir: PathBuf,
    history: tokio::sync::Mutex<Vec<ChatMessage>>,
}

// ---- Chat command ----

#[tauri::command]
async fn send_message(text: String, state: tauri::State<'_, AppState>) -> Result<String, String> {
    let auth = state
        .auth
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Not logged in. Click the user icon to authenticate.".to_string())?;

    let client = reqwest::Client::new();
    let tool_defs = tools::tool_definitions();
    let socket_path = state.socket_path.clone();
    let model = state.model.clone();

    let mut history = state.history.lock().await;

    history.push(ChatMessage::user(&text));

    // Agentic tool-call loop (mirrors repl.rs)
    loop {
        let reply = openai::chat_completion(&client, &auth, &model, &history, &tool_defs)
            .await
            .map_err(|e| e.to_string())?;

        if let Some(ref tool_calls) = reply.tool_calls {
            let calls = tool_calls.clone();
            history.push(reply);

            for tc in &calls {
                let args: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                let result =
                    match tools::dispatch_tool_call(&tc.function.name, &args, &socket_path).await {
                        Ok(r) => r,
                        Err(e) => format!("Error: {e}"),
                    };
                history.push(ChatMessage::tool_result(&tc.id, &result));
            }
            // Continue loop to let LLM process tool results
        } else {
            let response_text = reply.content.clone().unwrap_or_default();
            history.push(reply);
            return Ok(response_text);
        }
    }
}

// ---- Tracking commands ----

#[tauri::command]
async fn set_tracking_paused(
    paused: bool,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    let request_id = ulid::Ulid::new().to_string();
    let msg = Message::new(request_id, MessageKind::SetTrackingPaused { paused });
    let response = cronos_chat::daemon_client::send_request(msg, &state.socket_path)
        .await
        .map_err(|e| e.to_string())?;
    match response.kind {
        MessageKind::TrackingStatus { paused } => Ok(paused),
        MessageKind::Error { message, .. } => Err(message),
        _ => Err("Unexpected response from daemon".to_string()),
    }
}

#[tauri::command]
async fn kill_collectors() -> Result<(), String> {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("pkill")
            .args(["-f", "cronos-collect"])
            .output();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "cronos-collect-fs.exe"])
            .output();
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "cronos-collect-appmon.exe"])
            .output();
    }
    Ok(())
}

#[tauri::command]
async fn start_collectors() -> Result<(), String> {
    autostart::spawn_collector_if_absent();
    autostart::spawn_appmon_if_absent();
    Ok(())
}

// ---- Auth commands ----

#[tauri::command]
async fn login(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let result = oauth::login().await.map_err(|e| e.to_string())?;

    let (creds, label) = match result {
        oauth::LoginResult::ApiKey(ref key) => (
            credentials::StoredCredentials {
                api_key: Some(key.clone()),
                access_token: None,
                refresh_token: None,
                chatgpt_account_id: None,
            },
            "API Key".to_string(),
        ),
        oauth::LoginResult::OAuthTokens {
            ref access_token,
            ref refresh_token,
            ref chatgpt_account_id,
        } => (
            credentials::StoredCredentials {
                api_key: None,
                access_token: Some(access_token.clone()),
                refresh_token: Some(refresh_token.clone()),
                chatgpt_account_id: Some(chatgpt_account_id.clone()),
            },
            "ChatGPT OAuth".to_string(),
        ),
    };

    credentials::save(&state.config_dir, &creds).map_err(|e| e.to_string())?;

    let auth = match result {
        oauth::LoginResult::ApiKey(key) => Auth::ApiKey(key),
        oauth::LoginResult::OAuthTokens {
            access_token,
            chatgpt_account_id,
            ..
        } => Auth::ChatGpt {
            access_token,
            account_id: chatgpt_account_id,
        },
    };
    *state.auth.lock().unwrap() = Some(auth);

    Ok(label)
}

#[tauri::command]
async fn logout(state: tauri::State<'_, AppState>) -> Result<(), String> {
    credentials::remove_credentials(&state.config_dir).map_err(|e| e.to_string())?;
    *state.auth.lock().unwrap() = None;
    Ok(())
}

// ---- Status command ----

#[tauri::command]
async fn get_status(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let authenticated = state.auth.lock().unwrap().is_some();

    // Try to get daemon status
    let request_id = ulid::Ulid::new().to_string();
    let msg = Message::new(request_id, MessageKind::Status);
    let daemon_info = match cronos_chat::daemon_client::send_request(msg, &state.socket_path).await
    {
        Ok(response) => match response.kind {
            MessageKind::StatusResult { info } => Some(serde_json::json!({
                "entity_count": info.entity_count,
                "edge_count": info.edge_count,
                "event_count": info.event_count,
                "uptime_secs": info.uptime_secs,
                "connected_collectors": info.connected_collectors,
            })),
            _ => None,
        },
        Err(_) => None,
    };

    Ok(serde_json::json!({
        "authenticated": authenticated,
        "daemon_running": daemon_info.is_some(),
        "daemon": daemon_info,
        "tracking_paused": false,
    }))
}

// ---- Auth resolution (mirrors CLI chat/mod.rs) ----

fn resolve_auth(config: &CronosConfig, config_dir: &std::path::Path) -> Option<Auth> {
    if !config.ai.api_key.is_empty() {
        return Some(Auth::ApiKey(config.ai.api_key.clone()));
    }
    if let Ok(Some(creds)) = credentials::load(config_dir) {
        if let Some(ref key) = creds.api_key {
            if !key.is_empty() {
                return Some(Auth::ApiKey(key.clone()));
            }
        }
        if let (Some(token), Some(account_id)) = (&creds.access_token, &creds.chatgpt_account_id) {
            if !token.is_empty() && !account_id.is_empty() {
                return Some(Auth::ChatGpt {
                    access_token: token.clone(),
                    account_id: account_id.clone(),
                });
            }
        }
    }
    None
}

fn resolve_model(auth: &Option<Auth>, config: &CronosConfig) -> String {
    match auth {
        Some(Auth::ChatGpt { .. }) if config.ai.model == "gpt-4o" => {
            "gpt-5-codex-mini".to_string()
        }
        _ => config.ai.model.clone(),
    }
}

fn main() {
    let paths = CronosPaths::resolve().expect("resolving paths");
    let config = CronosConfig::load(&paths.config_file).unwrap_or_default();

    let auth = resolve_auth(&config, &paths.config_dir);
    let model = resolve_model(&auth, &config);
    let socket_path = if config.daemon.socket_path.is_empty() {
        paths.socket_file.clone()
    } else {
        PathBuf::from(&config.daemon.socket_path)
    };

    // Ensure daemon + collectors are running
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let _ = autostart::ensure_daemon(&socket_path).await;
    });
    autostart::spawn_collector_if_absent();
    autostart::spawn_appmon_if_absent();

    // System prompt
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
    let system_prompt = format!(
        "You are a personal developer assistant with access to the user's Cronos activity tracker. \
         Cronos tracks the user's app usage, window focus, and file changes. \
         Use cronos_day_summary to see what they did on a given day, and cronos_sessions for detailed session breakdowns. \
         Use cronos_recent for real-time file change events. \
         Use the provided tools to query the user's context and answer their questions. \
         Today's date and time is {now}. Answer concisely."
    );

    let state = AppState {
        auth: Mutex::new(auth),
        socket_path,
        model,
        config_dir: paths.config_dir,
        history: tokio::sync::Mutex::new(vec![ChatMessage::system(&system_prompt)]),
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            send_message,
            set_tracking_paused,
            kill_collectors,
            start_collectors,
            login,
            logout,
            get_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
