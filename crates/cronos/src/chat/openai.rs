use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Authentication mode for OpenAI API requests.
#[derive(Clone)]
pub enum Auth {
    /// Standard API key (works with api.openai.com).
    ApiKey(String),
    /// ChatGPT Plus/Pro OAuth (works with chatgpt.com backend API).
    ChatGpt {
        access_token: String,
        account_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

impl ChatMessage {
    pub fn system(content: &str) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            tool_call_id: None,
            tool_calls: None,
        }
    }

    pub fn user(content: &str) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_call_id: None,
            tool_calls: None,
        }
    }

    pub fn tool_result(tool_call_id: &str, content: &str) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: None,
        }
    }
}

pub async fn chat_completion(
    client: &reqwest::Client,
    auth: &Auth,
    model: &str,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
) -> Result<ChatMessage> {
    match auth {
        Auth::ApiKey(key) => chat_completions_api(client, key, model, messages, tools).await,
        Auth::ChatGpt {
            access_token,
            account_id,
        } => responses_api(client, access_token, account_id, model, messages, tools).await,
    }
}

// ---------------------------------------------------------------------------
// Standard Chat Completions API (api.openai.com, for API key users)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ChatCompletion {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChatMessage,
}

async fn chat_completions_api(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
) -> Result<ChatMessage> {
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "tools": tools,
        "tool_choice": "auto",
    });

    let resp = client
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .context("sending request to api.openai.com")?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        bail!("OpenAI API error ({}): {}", status, text);
    }

    let completion: ChatCompletion = resp.json().await.context("parsing OpenAI response")?;

    completion
        .choices
        .into_iter()
        .next()
        .map(|c| c.message)
        .ok_or_else(|| anyhow::anyhow!("OpenAI returned no choices"))
}

// ---------------------------------------------------------------------------
// Responses API (chatgpt.com/backend-api/codex, for ChatGPT Plus/Pro OAuth)
// ---------------------------------------------------------------------------

/// Convert our ChatMessage list to Responses API input format.
fn messages_to_responses_input(
    messages: &[ChatMessage],
) -> (Option<String>, Vec<serde_json::Value>) {
    let mut instructions: Option<String> = None;
    let mut input = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                // System messages become `instructions` in Responses API
                instructions = msg.content.clone();
            }
            "user" => {
                if let Some(ref text) = msg.content {
                    input.push(serde_json::json!({
                        "type": "message",
                        "role": "user",
                        "content": [{ "type": "input_text", "text": text }],
                    }));
                }
            }
            "assistant" => {
                // Text output goes in a message
                if let Some(ref text) = msg.content {
                    if !text.is_empty() {
                        input.push(serde_json::json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{ "type": "output_text", "text": text }],
                        }));
                    }
                }
                // Tool calls are top-level items, not nested in message content
                if let Some(ref tool_calls) = msg.tool_calls {
                    for tc in tool_calls {
                        input.push(serde_json::json!({
                            "type": "function_call",
                            "call_id": tc.id,
                            "name": tc.function.name,
                            "arguments": tc.function.arguments,
                        }));
                    }
                }
            }
            "tool" => {
                if let (Some(ref call_id), Some(ref output)) =
                    (&msg.tool_call_id, &msg.content)
                {
                    input.push(serde_json::json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": output,
                    }));
                }
            }
            _ => {}
        }
    }

    (instructions, input)
}

/// Convert Responses API tool definitions from Chat Completions format.
fn tools_to_responses_format(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .filter_map(|t| {
            let func = t.get("function")?;
            Some(serde_json::json!({
                "type": "function",
                "name": func.get("name")?,
                "description": func.get("description").unwrap_or(&serde_json::Value::Null),
                "parameters": func.get("parameters").unwrap_or(&serde_json::Value::Null),
            }))
        })
        .collect()
}

/// Parse a Responses API SSE stream and extract the assistant's reply.
///
/// The stream emits many event types. We use `response.completed` as the
/// primary source of truth since it contains the full response object with
/// all output items (text, function calls, etc.) fully populated.
async fn parse_responses_stream(resp: reqwest::Response) -> Result<ChatMessage> {
    let text = resp.text().await.context("reading response body")?;

    let mut output_text = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    // Parse SSE events — look for the response.completed event
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("data: ") {
            continue;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            break;
        }

        let event: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if event_type == "response.completed" {
            if let Some(response_obj) = event.get("response") {
                extract_from_response_object(response_obj, &mut output_text, &mut tool_calls);
            }
            break;
        }
    }

    // Fallback: if no response.completed event, try parsing as a single JSON object
    if output_text.is_empty() && tool_calls.is_empty() {
        if let Ok(response_obj) = serde_json::from_str::<serde_json::Value>(&text) {
            extract_from_response_object(&response_obj, &mut output_text, &mut tool_calls);
        }
    }

    Ok(ChatMessage {
        role: "assistant".to_string(),
        content: if output_text.is_empty() {
            None
        } else {
            Some(output_text)
        },
        tool_call_id: None,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
    })
}

/// Extract text and tool calls from a Responses API response object.
fn extract_from_response_object(
    obj: &serde_json::Value,
    output_text: &mut String,
    tool_calls: &mut Vec<ToolCall>,
) {
    if let Some(output) = obj.get("output").and_then(|v| v.as_array()) {
        for item in output {
            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match item_type {
                "message" => {
                    if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
                        for part in content {
                            if part.get("type").and_then(|v| v.as_str()) == Some("output_text") {
                                if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                                    if output_text.is_empty() {
                                        *output_text = t.to_string();
                                    }
                                }
                            }
                        }
                    }
                }
                "function_call" => {
                    let call_id = item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}")
                        .to_string();

                    tool_calls.push(ToolCall {
                        id: call_id,
                        kind: "function".to_string(),
                        function: FunctionCall { name, arguments },
                    });
                }
                _ => {}
            }
        }
    }
}

async fn responses_api(
    client: &reqwest::Client,
    access_token: &str,
    account_id: &str,
    model: &str,
    messages: &[ChatMessage],
    tools: &[serde_json::Value],
) -> Result<ChatMessage> {
    let (instructions, input) = messages_to_responses_input(messages);
    let resp_tools = tools_to_responses_format(tools);

    let mut body = serde_json::json!({
        "model": model,
        "input": input,
        "store": false,
        "stream": true,
    });

    if let Some(ref inst) = instructions {
        body["instructions"] = serde_json::Value::String(inst.clone());
    }
    if !resp_tools.is_empty() {
        body["tools"] = serde_json::Value::Array(resp_tools);
    }

    let resp = client
        .post("https://chatgpt.com/backend-api/codex/responses")
        .bearer_auth(access_token)
        .header("ChatGPT-Account-ID", account_id)
        .header("Accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .context("sending request to chatgpt.com")?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        bail!("ChatGPT API error ({}): {}", status, text);
    }

    parse_responses_stream(resp).await
}
