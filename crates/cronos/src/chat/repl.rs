use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::path::PathBuf;

use super::openai::{self, ChatMessage};
use super::tools;

pub async fn run_repl(api_key: String, model: String, socket_path: PathBuf) -> Result<()> {
    let client = reqwest::Client::new();
    let tool_defs = tools::tool_definitions();

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %Z");
    let system_prompt = format!(
        "You are a personal developer assistant with access to the user's Cronos activity tracker. \
         Cronos records file edits, project changes, and other developer activity on their local machine. \
         Use the provided tools to query the user's context and answer their questions. \
         Today's date and time is {now}. Answer concisely."
    );

    let mut history: Vec<ChatMessage> = vec![ChatMessage::system(&system_prompt)];
    let mut rl = DefaultEditor::new()?;

    println!("Cronos Chat (model: {model}). Type /quit or Ctrl+D to exit.\n");

    loop {
        let line = match rl.readline("You: ") {
            Ok(line) => line,
            Err(ReadlineError::Eof | ReadlineError::Interrupted) => break,
            Err(e) => return Err(e.into()),
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "/quit" || trimmed == "/exit" {
            break;
        }

        let _ = rl.add_history_entry(&line);
        history.push(ChatMessage::user(trimmed));

        // Agentic tool-call loop
        loop {
            let reply = openai::chat_completion(&client, &api_key, &model, &history, &tool_defs)
                .await?;

            if let Some(ref tool_calls) = reply.tool_calls {
                // Assistant made tool calls — push assistant message, dispatch each, continue loop
                let calls = tool_calls.clone();
                history.push(reply);

                for tc in &calls {
                    let args: serde_json::Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                    let result =
                        match tools::dispatch_tool_call(&tc.function.name, &args, &socket_path)
                            .await
                        {
                            Ok(r) => r,
                            Err(e) => format!("Error: {e}"),
                        };
                    history.push(ChatMessage::tool_result(&tc.id, &result));
                }
                // Continue the inner loop to let the LLM process tool results
            } else {
                // No tool calls — print text response and break inner loop
                let text = reply.content.as_deref().unwrap_or("");
                println!("\nCronos: {text}\n");
                history.push(reply);
                break;
            }
        }
    }

    Ok(())
}
