//! Built-in stdio Model Context Protocol (MCP) server (`manis-pocket mcp`).
//!
//! Exposes clipboard history inspection, search, safe item retrieval/export,
//! idempotent pinning, text addition, sync peer visibility, and validated
//! settings updates as JSON-Schema-typed MCP tools backed by `control_api`.

use std::io::{self, BufRead as _, Write as _};
use std::process::ExitCode;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app_update;
use crate::control_api::{
    self, AddTextParams, ControlRequest, CopyParams, DeleteParams, GetParams, ListParams,
    SearchParams, SetPinParams, UpdateSettingParams,
};

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

pub(crate) fn run_stdio(database: &str) -> ExitCode {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle_jsonrpc_line(database, &line)
            && (writeln!(out, "{reply}").is_err() || out.flush().is_err())
        {
            break;
        }
    }
    ExitCode::SUCCESS
}

pub(crate) fn handle_jsonrpc_line(database: &str, line: &str) -> Option<String> {
    let request = match serde_json::from_str::<JsonRpcRequest>(line) {
        Ok(req) => req,
        Err(error) => {
            return encode_response(JsonRpcResponse {
                jsonrpc: "2.0",
                id: Value::Null,
                result: None,
                error: Some(JsonRpcError {
                    code: -32700,
                    message: format!("parse error: {error}"),
                }),
            });
        }
    };

    // JSON-RPC notifications have no id and expect no reply.
    let id = request.id?;

    let response = match request.method.as_str() {
        "initialize" => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "manis-pocket",
                    "version": app_update::current_version()
                }
            })),
            error: None,
        },
        "ping" => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(json!({})),
            error: None,
        },
        "tools/list" => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(json!({ "tools": tool_definitions() })),
            error: None,
        },
        "tools/call" => handle_tool_call(id, database, &request.params),
        other => JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("method not found: {other}"),
            }),
        },
    };
    encode_response(response)
}

fn encode_response(response: JsonRpcResponse) -> Option<String> {
    serde_json::to_string(&response).ok()
}

fn handle_tool_call(id: Value, database: &str, params: &Value) -> JsonRpcResponse {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code: -32602,
                message: "tools/call requires string 'name'".to_owned(),
            }),
        };
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let control_request = match parse_tool_request(name, &args) {
        Ok(request) => request,
        Err(message) => {
            return JsonRpcResponse {
                jsonrpc: "2.0",
                id,
                result: Some(json!({
                    "content": [{ "type": "text", "text": message }],
                    "isError": true
                })),
                error: None,
            };
        }
    };

    let response = control_api::execute(database, &control_request);
    let text = serde_json::to_string_pretty(&response)
        .unwrap_or_else(|error| format!("{{\"ok\":false,\"error\":\"{error}\"}}"));
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(json!({
            "content": [{ "type": "text", "text": text }],
            "isError": !response.ok
        })),
        error: None,
    }
}

fn required_string(args: &Value, field: &str) -> Result<String, String> {
    args.get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing required string argument '{field}'"))
}

fn optional_string(args: &Value, field: &str) -> Option<String> {
    args.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn optional_usize(args: &Value, field: &str) -> Option<usize> {
    args.get(field)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn optional_bool(args: &Value, field: &str) -> bool {
    args.get(field).and_then(Value::as_bool).unwrap_or(false)
}

fn parse_tool_request(name: &str, args: &Value) -> Result<ControlRequest, String> {
    match name {
        "pocket_status" => Ok(ControlRequest::Status),
        "pocket_list_history" => Ok(ControlRequest::List(ListParams {
            limit: optional_usize(args, "limit"),
            kind: optional_string(args, "kind"),
            app: optional_string(args, "app"),
            pinned_only: optional_bool(args, "pinned_only"),
        })),
        "pocket_search_history" => Ok(ControlRequest::Search(SearchParams {
            query: required_string(args, "query")?,
            mode: optional_string(args, "mode"),
            kind: optional_string(args, "kind"),
            app: optional_string(args, "app"),
            limit: optional_usize(args, "limit"),
        })),
        "pocket_get_item" => Ok(ControlRequest::Get(GetParams {
            id: required_string(args, "id")?,
            max_chars: optional_usize(args, "max_chars"),
            export_image_path: optional_string(args, "export_image_path"),
        })),
        "pocket_copy_item" => Ok(ControlRequest::Copy(CopyParams {
            id: required_string(args, "id")?,
        })),
        "pocket_add_text" => Ok(ControlRequest::AddText(AddTextParams {
            text: required_string(args, "text")?,
            application: optional_string(args, "application"),
            pin: optional_bool(args, "pin"),
            copy_to_clipboard: optional_bool(args, "copy_to_clipboard"),
        })),
        "pocket_set_pin" => {
            let pinned = args
                .get("pinned")
                .and_then(Value::as_bool)
                .ok_or_else(|| "missing required boolean argument 'pinned'".to_owned())?;
            Ok(ControlRequest::SetPin(SetPinParams {
                id: required_string(args, "id")?,
                pinned,
            }))
        }
        "pocket_delete_item" => Ok(ControlRequest::Delete(DeleteParams {
            id: required_string(args, "id")?,
        })),
        "pocket_list_peers" => Ok(ControlRequest::Peers),
        "pocket_get_settings" => Ok(ControlRequest::SettingsGet),
        "pocket_update_setting" => {
            let key = required_string(args, "key")?;
            let value = args
                .get("value")
                .cloned()
                .ok_or_else(|| "missing required argument 'value'".to_owned())?;
            Ok(ControlRequest::SettingsSet(UpdateSettingParams {
                key,
                value,
            }))
        }
        other => Err(format!("unknown tool '{other}'")),
    }
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "pocket_status",
            "description": "Show Manis Pocket version, platform, database size, item/pin counts, and live P2P sync status.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "pocket_list_history",
            "description": "List recent clipboard history summaries (bounded previews; use pocket_get_item for full text or image export).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Maximum entries to return (default 20)" },
                    "kind": { "type": "string", "enum": ["text", "image", "file", "other"] },
                    "app": { "type": "string", "description": "Case-insensitive substring match against source application" },
                    "pinned_only": { "type": "boolean", "description": "Return only pinned items" }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "pocket_search_history",
            "description": "Search clipboard history using exact, fuzzy, regexp, or mixed matching.",
            "inputSchema": {
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string" },
                    "mode": { "type": "string", "enum": ["exact", "fuzzy", "regexp", "mixed"] },
                    "kind": { "type": "string", "enum": ["text", "image", "file", "other"] },
                    "app": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200 }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "pocket_get_item",
            "description": "Fetch full metadata and bounded text or image dimensions/SHA-256 for one clipboard item. Optionally export an image item to a file path.",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" },
                    "max_chars": { "type": "integer", "minimum": 1, "maximum": 262144, "description": "Maximum text characters to return (default 32768)" },
                    "export_image_path": { "type": "string", "description": "Optional file path where an image item's PNG bytes will be written" }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "pocket_copy_item",
            "description": "Write an existing clipboard history item (text, image, or file list) back to the system clipboard.",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "pocket_add_text",
            "description": "Add a text entry to Manis Pocket history (respects ignore rules and deduplication), optionally pinning or copying it to the system clipboard.",
            "inputSchema": {
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": { "type": "string" },
                    "application": { "type": "string" },
                    "pin": { "type": "boolean" },
                    "copy_to_clipboard": { "type": "boolean" }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "pocket_set_pin",
            "description": "Idempotently pin or unpin a clipboard item.",
            "inputSchema": {
                "type": "object",
                "required": ["id", "pinned"],
                "properties": {
                    "id": { "type": "string" },
                    "pinned": { "type": "boolean" }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "pocket_delete_item",
            "description": "Permanently delete a clipboard item by id.",
            "inputSchema": {
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": true, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "pocket_list_peers",
            "description": "List paired P2P sync devices and (when the desktop app is running) discovered LAN devices and pending pairing requests (without exposing raw PINs).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "pocket_get_settings",
            "description": "Show structured Manis Pocket settings (redacts raw ignore regexp patterns).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "pocket_update_setting",
            "description": "Update a validated Manis Pocket setting (searchMode, sortBy, pinsPosition, popupPosition, historySize, unlimitedHistory, pasteByDefault, saveImages, saveText, showFooter, showSearch, showTitle, syncEnabled).",
            "inputSchema": {
                "type": "object",
                "required": ["key", "value"],
                "properties": {
                    "key": { "type": "string" },
                    "value": {}
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_initialize_ping_tools_list_and_notifications() {
        let init = handle_jsonrpc_line(
            ":memory:",
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        )
        .expect("initialize reply");
        let parsed: Value = serde_json::from_str(&init).expect("valid json");
        assert_eq!(parsed["result"]["serverInfo"]["name"], "manis-pocket");

        let ping = handle_jsonrpc_line(":memory:", r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#)
            .expect("ping reply");
        let parsed: Value = serde_json::from_str(&ping).expect("valid json");
        assert!(parsed["error"].is_null());

        let tools = handle_jsonrpc_line(
            ":memory:",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
        )
        .expect("tools reply");
        let parsed: Value = serde_json::from_str(&tools).expect("valid json");
        assert_eq!(parsed["result"]["tools"].as_array().unwrap().len(), 11);

        // Notifications have no id and produce no reply.
        assert!(
            handle_jsonrpc_line(
                ":memory:",
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
            )
            .is_none()
        );
    }

    #[test]
    fn executes_tools_and_reports_errors_cleanly() {
        let dir =
            std::env::temp_dir().join(format!("manis-pocket-mcp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let db = dir.join("manispocket.db").to_string_lossy().into_owned();

        let add = handle_jsonrpc_line(
            &db,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"pocket_add_text","arguments":{"text":"mcp hello","pin":true}}}"#,
        )
        .expect("add reply");
        let parsed: Value = serde_json::from_str(&add).expect("valid json");
        assert_eq!(parsed["result"]["isError"], false);

        let search = handle_jsonrpc_line(
            &db,
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"pocket_search_history","arguments":{"query":"hello"}}}"#,
        )
        .expect("search reply");
        let parsed: Value = serde_json::from_str(&search).expect("valid json");
        assert_eq!(parsed["result"]["isError"], false);
        assert!(
            parsed["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("mcp hello")
        );

        let unknown = handle_jsonrpc_line(
            &db,
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"no_such_tool","arguments":{}}}"#,
        )
        .expect("error reply");
        let parsed: Value = serde_json::from_str(&unknown).expect("valid json");
        assert_eq!(parsed["result"]["isError"], true);

        let malformed = handle_jsonrpc_line(&db, "{not json").expect("parse error");
        let parsed: Value = serde_json::from_str(&malformed).expect("valid json");
        assert_eq!(parsed["error"]["code"], -32700);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
