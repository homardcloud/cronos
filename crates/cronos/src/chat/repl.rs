use anyhow::Result;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    DefaultTerminal, Frame,
};
use std::io;
use std::path::PathBuf;
use tokio::sync::mpsc;

use cronos_chat::openai::{self, Auth, ChatMessage};
use cronos_chat::tools;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const AMBER: Color = Color::Rgb(255, 179, 0);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum MessageEntry {
    User(String),
    Assistant(String),
    ToolProgress(String),
    Error(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppState {
    Idle,
    Loading,
}

enum BackendMsg {
    ToolProgress(String),
    AssistantResponse {
        text: String,
        updated_history: Vec<ChatMessage>,
    },
    Error(String),
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct App {
    messages: Vec<MessageEntry>,
    input: String,
    cursor: usize,
    scroll_offset: u16,
    pinned_to_bottom: bool,
    state: AppState,
    should_quit: bool,
    model: String,
    spinner_tick: usize,
    // Chat engine state
    history: Vec<ChatMessage>,
    auth: Auth,
    client: reqwest::Client,
    tool_defs: Vec<serde_json::Value>,
    socket_path: PathBuf,
}

impl App {
    fn new(auth: Auth, model: String, socket_path: PathBuf) -> Self {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
        let system_prompt = format!(
            "You are a personal developer assistant with access to the user's Cronos activity tracker. \
             Cronos tracks the user's app usage, window focus, and file changes. \
             Use cronos_day_summary to see what they did on a given day, and cronos_sessions for detailed session breakdowns. \
             Use cronos_recent for real-time file change events. \
             Use the provided tools to query the user's context and answer their questions. \
             Today's date and time is {now}. Answer concisely."
        );

        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor: 0,
            scroll_offset: 0,
            pinned_to_bottom: true,
            state: AppState::Idle,
            should_quit: false,
            model,
            spinner_tick: 0,
            history: vec![ChatMessage::system(&system_prompt)],
            auth,
            client: reqwest::Client::new(),
            tool_defs: tools::tool_definitions(),
            socket_path,
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal guard (restores terminal on drop/panic)
// ---------------------------------------------------------------------------

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run_repl(auth: Auth, model: String, socket_path: PathBuf) -> Result<()> {
    // Set up panic hook to restore terminal
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let _guard = TerminalGuard;

    let terminal = ratatui::init();
    let result = run_app(terminal, auth, model, socket_path).await;

    ratatui::restore();
    result
}

async fn run_app(
    mut terminal: DefaultTerminal,
    auth: Auth,
    model: String,
    socket_path: PathBuf,
) -> Result<()> {
    let mut app = App::new(auth, model, socket_path);
    let (backend_tx, mut backend_rx) = mpsc::unbounded_channel::<BackendMsg>();
    let mut event_stream = EventStream::new();
    let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(200));

    // Initial draw
    terminal.draw(|f| draw(f, &app))?;

    loop {
        if app.should_quit {
            break;
        }

        tokio::select! {
            // Terminal events
            maybe_event = event_stream.next() => {
                if let Some(Ok(evt)) = maybe_event {
                    if let Event::Key(key) = evt {
                        handle_key_event(&mut app, key, &backend_tx);
                    }
                }
            }
            // Backend messages
            Some(msg) = backend_rx.recv() => {
                handle_backend_msg(&mut app, msg);
            }
            // Spinner tick
            _ = ticker.tick() => {
                if app.state == AppState::Loading {
                    app.spinner_tick += 1;
                }
            }
        }

        terminal.draw(|f| draw(f, &app))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Key event handling
// ---------------------------------------------------------------------------

fn handle_key_event(app: &mut App, key: KeyEvent, backend_tx: &mpsc::UnboundedSender<BackendMsg>) {
    match key.code {
        // Quit
        KeyCode::Char('c') | KeyCode::Char('d')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.should_quit = true;
        }

        // Submit
        KeyCode::Enter if app.state == AppState::Idle => {
            let input = app.input.trim().to_string();
            if input.is_empty() {
                return;
            }
            if input == "/quit" || input == "/exit" {
                app.should_quit = true;
                return;
            }
            app.messages.push(MessageEntry::User(input.clone()));
            app.input.clear();
            app.cursor = 0;
            app.pinned_to_bottom = true;
            app.state = AppState::Loading;
            app.spinner_tick = 0;

            // Spawn background task
            spawn_chat_task(app, &input, backend_tx.clone());
        }

        // Text input (only when idle)
        KeyCode::Char(c) if app.state == AppState::Idle => {
            app.input.insert(app.cursor, c);
            app.cursor += c.len_utf8();
        }
        KeyCode::Backspace if app.state == AppState::Idle => {
            if app.cursor > 0 {
                // Find the previous char boundary
                let prev = app.input[..app.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                app.input.drain(prev..app.cursor);
                app.cursor = prev;
            }
        }
        KeyCode::Delete if app.state == AppState::Idle => {
            if app.cursor < app.input.len() {
                let next = app.input[app.cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| app.cursor + i)
                    .unwrap_or(app.input.len());
                app.input.drain(app.cursor..next);
            }
        }
        KeyCode::Left if app.state == AppState::Idle => {
            if app.cursor > 0 {
                app.cursor = app.input[..app.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
            }
        }
        KeyCode::Right if app.state == AppState::Idle => {
            if app.cursor < app.input.len() {
                app.cursor = app.input[app.cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| app.cursor + i)
                    .unwrap_or(app.input.len());
            }
        }
        KeyCode::Home if app.state == AppState::Idle => {
            app.cursor = 0;
        }
        KeyCode::End if app.state == AppState::Idle => {
            app.cursor = app.input.len();
        }

        // Scrolling
        KeyCode::PageUp => {
            app.scroll_offset = app.scroll_offset.saturating_add(10);
            app.pinned_to_bottom = false;
        }
        KeyCode::PageDown => {
            app.scroll_offset = app.scroll_offset.saturating_sub(10);
            if app.scroll_offset == 0 {
                app.pinned_to_bottom = true;
            }
        }
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.scroll_offset = app.scroll_offset.saturating_add(1);
            app.pinned_to_bottom = false;
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.scroll_offset = app.scroll_offset.saturating_sub(1);
            if app.scroll_offset == 0 {
                app.pinned_to_bottom = true;
            }
        }

        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Background chat task
// ---------------------------------------------------------------------------

fn spawn_chat_task(
    app: &mut App,
    user_input: &str,
    tx: mpsc::UnboundedSender<BackendMsg>,
) {
    let mut history = app.history.clone();
    history.push(ChatMessage::user(user_input));

    let client = app.client.clone();
    let auth = app.auth.clone();
    let model = app.model.clone();
    let tool_defs = app.tool_defs.clone();
    let socket_path = app.socket_path.clone();

    tokio::spawn(async move {
        let result = run_agentic_loop(
            &client,
            &auth,
            &model,
            &mut history,
            &tool_defs,
            &socket_path,
            &tx,
        )
        .await;

        match result {
            Ok(text) => {
                let _ = tx.send(BackendMsg::AssistantResponse {
                    text,
                    updated_history: history,
                });
            }
            Err(e) => {
                let _ = tx.send(BackendMsg::Error(format!("{e:#}")));
            }
        }
    });
}

async fn run_agentic_loop(
    client: &reqwest::Client,
    auth: &Auth,
    model: &str,
    history: &mut Vec<ChatMessage>,
    tool_defs: &[serde_json::Value],
    socket_path: &PathBuf,
    tx: &mpsc::UnboundedSender<BackendMsg>,
) -> Result<String> {
    loop {
        let reply = openai::chat_completion(client, auth, model, history, tool_defs).await?;

        if let Some(ref tool_calls) = reply.tool_calls {
            let calls = tool_calls.clone();
            history.push(reply);

            for tc in &calls {
                let _ = tx.send(BackendMsg::ToolProgress(format!(
                    "Calling {}...",
                    tc.function.name
                )));

                let args: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                let result =
                    match tools::dispatch_tool_call(&tc.function.name, &args, socket_path).await {
                        Ok(r) => r,
                        Err(e) => format!("Error: {e}"),
                    };
                history.push(ChatMessage::tool_result(&tc.id, &result));
            }
        } else {
            let text = reply.content.as_deref().unwrap_or("").to_string();
            history.push(reply);
            return Ok(text);
        }
    }
}

// ---------------------------------------------------------------------------
// Handle backend messages
// ---------------------------------------------------------------------------

fn handle_backend_msg(app: &mut App, msg: BackendMsg) {
    match msg {
        BackendMsg::ToolProgress(text) => {
            app.messages.push(MessageEntry::ToolProgress(text));
            app.pinned_to_bottom = true;
        }
        BackendMsg::AssistantResponse {
            text,
            updated_history,
        } => {
            app.messages.push(MessageEntry::Assistant(text));
            app.history = updated_history;
            app.state = AppState::Idle;
            app.pinned_to_bottom = true;
        }
        BackendMsg::Error(text) => {
            app.messages.push(MessageEntry::Error(text));
            app.state = AppState::Idle;
            app.pinned_to_bottom = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    let chunks = Layout::vertical([
        Constraint::Length(3), // Header
        Constraint::Fill(1),  // Conversation
        Constraint::Length(3), // Input
    ])
    .split(area);

    // -- Header --
    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(AMBER))
        .title(Span::styled(
            " cronos ",
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(ratatui::layout::Alignment::Left);

    let model_text = Span::styled(
        format!(" {} ", app.model),
        Style::default().fg(Color::DarkGray),
    );
    let header = Paragraph::new(Line::from(vec![model_text]))
        .block(header_block)
        .alignment(ratatui::layout::Alignment::Right);
    f.render_widget(header, chunks[0]);

    // -- Conversation --
    let conv_block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner_width = chunks[1].width.saturating_sub(2) as usize; // account for borders
    let mut lines: Vec<Line> = Vec::new();

    for entry in &app.messages {
        match entry {
            MessageEntry::User(text) => {
                lines.push(Line::from(""));
                let prefix = Span::styled(
                    "You: ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                );
                // Word-wrap user text
                let wrapped = wrap_text(text, inner_width.saturating_sub(5));
                for (i, wline) in wrapped.iter().enumerate() {
                    if i == 0 {
                        lines.push(Line::from(vec![
                            prefix.clone(),
                            Span::raw(wline.clone()),
                        ]));
                    } else {
                        lines.push(Line::from(vec![
                            Span::raw("     "),
                            Span::raw(wline.clone()),
                        ]));
                    }
                }
            }
            MessageEntry::Assistant(text) => {
                lines.push(Line::from(""));
                let wrapped = wrap_text(text, inner_width);
                for wline in &wrapped {
                    lines.push(Line::from(Span::raw(wline.clone())));
                }
            }
            MessageEntry::ToolProgress(text) => {
                lines.push(Line::from(Span::styled(
                    format!("  [{text}]"),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )));
            }
            MessageEntry::Error(text) => {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("Error: {text}"),
                    Style::default().fg(Color::Red),
                )));
            }
        }
    }

    // Loading indicator
    if app.state == AppState::Loading {
        let dots = ".".repeat((app.spinner_tick % 4) + 1);
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  Thinking{dots}"),
            Style::default()
                .fg(AMBER)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    let total_lines = lines.len() as u16;
    let visible_height = chunks[1].height.saturating_sub(2); // borders

    // Auto-scroll: if pinned, set scroll so bottom is visible
    let scroll = if app.pinned_to_bottom {
        total_lines.saturating_sub(visible_height)
    } else {
        // Clamp scroll offset
        let max_scroll = total_lines.saturating_sub(visible_height);
        max_scroll.saturating_sub(app.scroll_offset)
    };

    let conversation = Paragraph::new(lines)
        .block(conv_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(conversation, chunks[1]);

    // -- Input --
    let input_border_color = if app.state == AppState::Idle {
        AMBER
    } else {
        Color::DarkGray
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(input_border_color));

    let prompt_span = Span::styled(
        "> ",
        Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
    );

    let input_text = if app.state == AppState::Loading {
        Span::styled(
            "waiting...",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )
    } else {
        Span::raw(app.input.as_str())
    };

    let input_line = Line::from(vec![prompt_span, input_text]);
    let input_widget = Paragraph::new(input_line).block(input_block);
    f.render_widget(input_widget, chunks[2]);

    // Set cursor position (only when idle)
    if app.state == AppState::Idle {
        // Calculate visual cursor position (chars before cursor)
        let visual_pos = app.input[..app.cursor].chars().count();
        f.set_cursor_position((
            chunks[2].x + 1 + 2 + visual_pos as u16, // border + "> " + chars
            chunks[2].y + 1,                          // border
        ));
    }
}

// ---------------------------------------------------------------------------
// Text wrapping helper
// ---------------------------------------------------------------------------

fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            result.push(String::new());
            continue;
        }

        let words: Vec<&str> = paragraph.split_whitespace().collect();
        if words.is_empty() {
            result.push(String::new());
            continue;
        }

        let mut current_line = String::new();
        for word in words {
            if current_line.is_empty() {
                if word.len() > max_width {
                    // Word is longer than line width — force break
                    let mut remaining = word;
                    while remaining.len() > max_width {
                        let (chunk, rest) = remaining.split_at(max_width);
                        result.push(chunk.to_string());
                        remaining = rest;
                    }
                    current_line = remaining.to_string();
                } else {
                    current_line = word.to_string();
                }
            } else if current_line.len() + 1 + word.len() <= max_width {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                result.push(current_line);
                current_line = word.to_string();
            }
        }
        if !current_line.is_empty() {
            result.push(current_line);
        }
    }

    if result.is_empty() {
        result.push(String::new());
    }
    result
}
