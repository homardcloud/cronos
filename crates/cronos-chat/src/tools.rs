use anyhow::{bail, Result};
use cronos_model::EntityId;
use cronos_proto::*;
use std::path::Path;

use crate::daemon_client;

/// Return the OpenAI function-calling tool schema for the 5 Cronos tools.
pub fn tool_definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "cronos_recent",
                "description": "Get the most recent N activity events from the user's Cronos tracker.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of recent events to return (default 20)"
                        }
                    }
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "cronos_search",
                "description": "Full-text search across entity names in the user's Cronos activity tracker.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                            "description": "Search query text"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of results (default 20)"
                        }
                    },
                    "required": ["text"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "cronos_timeline",
                "description": "Get events between two timestamps (milliseconds since Unix epoch).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "from_ms": {
                            "type": "integer",
                            "description": "Start timestamp in milliseconds since Unix epoch"
                        },
                        "to_ms": {
                            "type": "integer",
                            "description": "End timestamp in milliseconds since Unix epoch"
                        }
                    },
                    "required": ["from_ms", "to_ms"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "cronos_related",
                "description": "Traverse the context graph from an entity to find related entities.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "entity_id": {
                            "type": "string",
                            "description": "ULID of the entity to start traversal from"
                        },
                        "depth": {
                            "type": "integer",
                            "description": "Traversal depth (default 2)"
                        }
                    },
                    "required": ["entity_id"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "cronos_status",
                "description": "Get daemon status including entity count, edge count, event count, and uptime.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "cronos_sessions",
                "description": "Get activity sessions for a time range. Sessions group consecutive app usage — shows which app, how long, window titles, and category (coding/communication/browsing/etc).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "from_ms": {
                            "type": "integer",
                            "description": "Start timestamp in ms since epoch"
                        },
                        "to_ms": {
                            "type": "integer",
                            "description": "End timestamp in ms since epoch"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max sessions to return (default 50)"
                        }
                    },
                    "required": ["from_ms", "to_ms"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "cronos_day_summary",
                "description": "Get a structured summary of the user's activity for a specific date — total hours tracked, breakdown by category (coding, communication, browsing, etc) with apps and durations.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "date": {
                            "type": "string",
                            "description": "Date in YYYY-MM-DD format"
                        }
                    },
                    "required": ["date"]
                }
            }
        }),
    ]
}

/// Dispatch a tool call to the Cronos daemon and return the JSON result string.
pub async fn dispatch_tool_call(
    name: &str,
    args: &serde_json::Value,
    socket_path: &Path,
) -> Result<String> {
    let request_id = ulid::Ulid::new().to_string();

    let msg = match name {
        "cronos_recent" => {
            let limit = args["limit"].as_u64().unwrap_or(20) as u32;
            Message::new(
                request_id,
                MessageKind::Query {
                    query: QueryRequest {
                        kind: QueryKind::Recent { limit },
                    },
                },
            )
        }
        "cronos_search" => {
            let text = args["text"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let limit = args["limit"].as_u64().unwrap_or(20) as u32;
            Message::new(
                request_id,
                MessageKind::Query {
                    query: QueryRequest {
                        kind: QueryKind::Search { text, limit },
                    },
                },
            )
        }
        "cronos_timeline" => {
            let from = args["from_ms"].as_i64().unwrap_or(0);
            let to = args["to_ms"].as_i64().unwrap_or(0);
            Message::new(
                request_id,
                MessageKind::Query {
                    query: QueryRequest {
                        kind: QueryKind::Timeline { from, to },
                    },
                },
            )
        }
        "cronos_related" => {
            let entity_id_str = args["entity_id"]
                .as_str()
                .unwrap_or("");
            let ulid = ulid::Ulid::from_string(entity_id_str)
                .map_err(|e| anyhow::anyhow!("invalid entity ID '{}': {}", entity_id_str, e))?;
            let depth = args["depth"].as_u64().unwrap_or(2) as u8;
            Message::new(
                request_id,
                MessageKind::Query {
                    query: QueryRequest {
                        kind: QueryKind::Related {
                            entity_id: EntityId(ulid),
                            depth,
                        },
                    },
                },
            )
        }
        "cronos_status" => Message::new(request_id, MessageKind::Status),
        "cronos_sessions" => {
            let from = args["from_ms"].as_i64().unwrap_or(0);
            let to = args["to_ms"].as_i64().unwrap_or(0);
            let limit = args["limit"].as_u64().unwrap_or(50) as u32;
            Message::new(
                request_id,
                MessageKind::Query {
                    query: QueryRequest {
                        kind: QueryKind::Sessions { from, to, limit },
                    },
                },
            )
        }
        "cronos_day_summary" => {
            let date = args["date"]
                .as_str()
                .unwrap_or("")
                .to_string();
            Message::new(
                request_id,
                MessageKind::Query {
                    query: QueryRequest {
                        kind: QueryKind::DaySummary { date },
                    },
                },
            )
        }
        _ => bail!("unknown tool: {}", name),
    };

    let response = daemon_client::send_request(msg, socket_path).await?;
    serde_json::to_string_pretty(&response).map_err(Into::into)
}
