//! Agent-first CLI frontend sharing the `manis-pocket` binary with the GPUI app.
//!
//! Invoking `manis-pocket` with no arguments (or with GUI flags like `--toggle`,
//! `--settings`, or a bare `--database <path>`) keeps the desktop behavior
//! unchanged. Passing a subcommand runs headless against `<data_dir>/agent.sock`
//! (falling back to the local SQLite database when offline) and exits without
//! opening a window.

use std::io::Read as _;
use std::process::ExitCode;

use serde_json::Value;

use crate::app_update;
use crate::control_api::{
    self, AddTextParams, ControlRequest, ControlResponse, CopyParams, DeleteParams, GetParams,
    ListParams, SearchParams, SetPinParams, UpdateSettingParams,
};
use crate::mcp;

const HELP_TEXT: &str = "\
Manis Pocket — clipboard history, P2P sync & Agent CLI

USAGE:
  manis-pocket                              Launch desktop app
  manis-pocket <COMMAND> [OPTIONS]

COMMANDS:
  status                                    Show version, database, item/pin counts, and sync status
  list                                      List recent clipboard history summaries
  search <query>                            Search clipboard history
  get <id>                                  Inspect one entry (bounded text or image metadata/export)
  copy <id>                                 Copy an entry back to the system clipboard
  add <text|->                              Add text to history (pass '-' to read stdin)
  pin <id>                                  Pin an entry (idempotent)
  unpin <id>                                Unpin an entry (idempotent)
  delete <id>                               Delete an entry by id
  peers                                     List paired/discovered P2P devices
  settings show                             Show structured settings
  settings set <key> <value>                Update a validated setting
  mcp                                       Start stdio MCP server for AI agents

COMMON OPTIONS:
  --json                                    Emit machine-readable JSON output
  --database <path>                         Override SQLite database / socket directory
  -V, --version                             Print version
  -h, --help                                Print help

LIST / SEARCH OPTIONS:
  --limit <n>                               Max items to return (default 20, max 200)
  --kind <text|image|file|other>            Filter by content kind
  --app <substr>                            Filter by source application substring
  --pinned                                  List only pinned items (`list` only)
  --mode <exact|fuzzy|regexp|mixed>         Search mode (`search` only)

GET / ADD OPTIONS:
  --max-chars <n>                           Max text characters to return (`get`, default 32768)
  --export-image <path>                     Export PNG bytes to file (`get`)
  --pin                                     Pin the added text (`add`)
  --copy                                    Also copy added text to system clipboard (`add`)
  --app <name>                              Source application label (`add`)";

#[derive(Clone, Debug, Eq, PartialEq)]
enum CliAction {
    Version,
    Help,
    Mcp {
        database: Option<String>,
    },
    Control {
        request: ControlRequest,
        json: bool,
        database: Option<String>,
    },
}

const SUBCOMMANDS: [&str; 12] = [
    "status", "list", "search", "get", "copy", "add", "pin", "unpin", "delete", "peers",
    "settings", "mcp",
];

/// Checks process arguments and runs the headless CLI if an agent subcommand
/// (or `--help` / `--version`) was supplied.
///
/// Returns `None` when the process should continue into normal GUI / legacy
/// flag handling in `main`.
#[must_use]
pub(crate) fn try_run_from_env() -> Option<ExitCode> {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    if raw_args.is_empty() || raw_args.iter().all(|arg| arg.starts_with("-psn_")) {
        return None;
    }

    let args: Vec<&str> = raw_args.iter().map(String::as_str).collect();
    if !is_cli_invocation(&args) {
        return None;
    }

    match parse_args(&args) {
        Ok(CliAction::Version) => {
            println!("Manis Pocket {}", app_update::current_version());
            Some(ExitCode::SUCCESS)
        }
        Ok(CliAction::Help) => {
            println!("{HELP_TEXT}");
            Some(ExitCode::SUCCESS)
        }
        Ok(CliAction::Mcp { database }) => {
            let db = database.unwrap_or_else(crate::storage_path);
            Some(mcp::run_stdio(&db))
        }
        Ok(CliAction::Control {
            request,
            json,
            database,
        }) => {
            let db = database.unwrap_or_else(crate::storage_path);
            Some(run_control(&db, &request, json))
        }
        Err(message) => {
            eprintln!("error: {message}\n\nRun `manis-pocket --help` for usage.");
            Some(ExitCode::from(2))
        }
    }
}

/// Distinguishes Agent CLI subcommands from legacy GUI flags (`--toggle`,
/// `--settings`, `--copy`, `--pair`, bare `--database <path>`, etc.).
fn is_cli_invocation(args: &[&str]) -> bool {
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "-h" | "--help" | "help" | "-V" | "--version" => return true,
            "--database" => index += 2,
            "--json" => index += 1,
            token if SUBCOMMANDS.contains(&token) => return true,
            _ => return false,
        }
    }
    false
}

fn run_control(database: &str, request: &ControlRequest, json_output: bool) -> ExitCode {
    let response = control_api::execute(database, request);
    if json_output {
        match serde_json::to_string_pretty(&response) {
            Ok(encoded) => println!("{encoded}"),
            Err(error) => {
                eprintln!("error: failed to serialize response: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print_human(request, &response);
    }
    if response.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn parse_args(args: &[&str]) -> Result<CliAction, String> {
    let mut database = None;
    let mut json = false;
    let mut index = 0;

    while index < args.len() {
        match args[index] {
            "-V" | "--version" => return Ok(CliAction::Version),
            "-h" | "--help" | "help" => return Ok(CliAction::Help),
            "--database" => {
                database = Some(next_value(args, &mut index, "--database")?.to_owned());
                index += 1;
            }
            "--json" => {
                json = true;
                index += 1;
            }
            command => {
                let rest = &args[index + 1..];
                return parse_subcommand(command, rest, database, json);
            }
        }
    }
    Ok(CliAction::Help)
}

fn parse_subcommand(
    command: &str,
    rest: &[&str],
    mut database: Option<String>,
    mut json: bool,
) -> Result<CliAction, String> {
    match command {
        "mcp" => {
            let mut index = 0;
            while index < rest.len() {
                match rest[index] {
                    "--database" => {
                        database = Some(next_value(rest, &mut index, "--database")?.to_owned());
                    }
                    "-h" | "--help" => return Ok(CliAction::Help),
                    other => return Err(format!("unexpected argument '{other}' for mcp")),
                }
                index += 1;
            }
            Ok(CliAction::Mcp { database })
        }
        "status" => parse_no_args(ControlRequest::Status, rest, database, json),
        "peers" => parse_no_args(ControlRequest::Peers, rest, database, json),
        "list" => parse_list(rest, database, json),
        "search" => parse_search(rest, database, json),
        "get" => parse_get(rest, database, json),
        "copy" => parse_id_only(rest, database, json, |id| {
            ControlRequest::Copy(CopyParams { id })
        }),
        "pin" => parse_id_only(rest, database, json, |id| {
            ControlRequest::SetPin(SetPinParams { id, pinned: true })
        }),
        "unpin" => parse_id_only(rest, database, json, |id| {
            ControlRequest::SetPin(SetPinParams { id, pinned: false })
        }),
        "delete" => parse_id_only(rest, database, json, |id| {
            ControlRequest::Delete(DeleteParams { id })
        }),
        "add" => parse_add(rest, database, json),
        "settings" => parse_settings(rest, database, &mut json),
        other => Err(format!("unknown command '{other}'")),
    }
}

fn parse_no_args(
    request: ControlRequest,
    args: &[&str],
    mut database: Option<String>,
    mut json: bool,
) -> Result<CliAction, String> {
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--json" => json = true,
            "--database" => {
                database = Some(next_value(args, &mut index, "--database")?.to_owned());
            }
            "-h" | "--help" => return Ok(CliAction::Help),
            other => return Err(format!("unexpected argument '{other}'")),
        }
        index += 1;
    }
    Ok(CliAction::Control {
        request,
        json,
        database,
    })
}

fn parse_id_only(
    args: &[&str],
    mut database: Option<String>,
    mut json: bool,
    build: impl FnOnce(String) -> ControlRequest,
) -> Result<CliAction, String> {
    let mut id = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--json" => json = true,
            "--database" => {
                database = Some(next_value(args, &mut index, "--database")?.to_owned());
            }
            "-h" | "--help" => return Ok(CliAction::Help),
            arg if arg.starts_with('-') => return Err(format!("unknown option '{arg}'")),
            positional if id.is_none() => id = Some(positional.to_owned()),
            extra => return Err(format!("unexpected argument '{extra}'")),
        }
        index += 1;
    }
    let id = id.ok_or_else(|| "missing required <id> argument".to_owned())?;
    Ok(CliAction::Control {
        request: build(id),
        json,
        database,
    })
}

fn parse_list(
    args: &[&str],
    mut database: Option<String>,
    mut json: bool,
) -> Result<CliAction, String> {
    let mut params = ListParams::default();
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--json" => json = true,
            "--database" => {
                database = Some(next_value(args, &mut index, "--database")?.to_owned());
            }
            "--limit" => {
                let raw = next_value(args, &mut index, "--limit")?;
                params.limit = Some(
                    raw.parse::<usize>()
                        .map_err(|_| format!("invalid --limit '{raw}'"))?,
                );
            }
            "--kind" => {
                params.kind = Some(next_value(args, &mut index, "--kind")?.to_owned());
            }
            "--app" => {
                params.app = Some(next_value(args, &mut index, "--app")?.to_owned());
            }
            "--pinned" => params.pinned_only = true,
            "-h" | "--help" => return Ok(CliAction::Help),
            other => return Err(format!("unexpected argument '{other}' for list")),
        }
        index += 1;
    }
    Ok(CliAction::Control {
        request: ControlRequest::List(params),
        json,
        database,
    })
}

fn parse_search(
    args: &[&str],
    mut database: Option<String>,
    mut json: bool,
) -> Result<CliAction, String> {
    let mut query = None;
    let mut mode = None;
    let mut kind = None;
    let mut app = None;
    let mut limit = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--json" => json = true,
            "--database" => {
                database = Some(next_value(args, &mut index, "--database")?.to_owned());
            }
            "--mode" => mode = Some(next_value(args, &mut index, "--mode")?.to_owned()),
            "--kind" => kind = Some(next_value(args, &mut index, "--kind")?.to_owned()),
            "--app" => app = Some(next_value(args, &mut index, "--app")?.to_owned()),
            "--limit" => {
                let raw = next_value(args, &mut index, "--limit")?;
                limit = Some(
                    raw.parse::<usize>()
                        .map_err(|_| format!("invalid --limit '{raw}'"))?,
                );
            }
            "-h" | "--help" => return Ok(CliAction::Help),
            arg if arg.starts_with('-') => {
                return Err(format!("unknown option '{arg}' for search"));
            }
            positional if query.is_none() => query = Some(positional.to_owned()),
            extra => return Err(format!("unexpected argument '{extra}' for search")),
        }
        index += 1;
    }
    let query = query.ok_or_else(|| "search requires a <query> argument".to_owned())?;
    Ok(CliAction::Control {
        request: ControlRequest::Search(SearchParams {
            query,
            mode,
            kind,
            app,
            limit,
        }),
        json,
        database,
    })
}

fn parse_get(
    args: &[&str],
    mut database: Option<String>,
    mut json: bool,
) -> Result<CliAction, String> {
    let mut id = None;
    let mut max_chars = None;
    let mut export_image_path = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--json" => json = true,
            "--database" => {
                database = Some(next_value(args, &mut index, "--database")?.to_owned());
            }
            "--max-chars" => {
                let raw = next_value(args, &mut index, "--max-chars")?;
                max_chars = Some(
                    raw.parse::<usize>()
                        .map_err(|_| format!("invalid --max-chars '{raw}'"))?,
                );
            }
            "--export-image" => {
                export_image_path =
                    Some(next_value(args, &mut index, "--export-image")?.to_owned());
            }
            "-h" | "--help" => return Ok(CliAction::Help),
            arg if arg.starts_with('-') => return Err(format!("unknown option '{arg}' for get")),
            positional if id.is_none() => id = Some(positional.to_owned()),
            extra => return Err(format!("unexpected argument '{extra}' for get")),
        }
        index += 1;
    }
    let id = id.ok_or_else(|| "get requires an <id> argument".to_owned())?;
    Ok(CliAction::Control {
        request: ControlRequest::Get(GetParams {
            id,
            max_chars,
            export_image_path,
        }),
        json,
        database,
    })
}

fn parse_add(
    args: &[&str],
    mut database: Option<String>,
    mut json: bool,
) -> Result<CliAction, String> {
    let mut raw_text = None;
    let mut application = None;
    let mut pin = false;
    let mut copy_to_clipboard = false;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--json" => json = true,
            "--database" => {
                database = Some(next_value(args, &mut index, "--database")?.to_owned());
            }
            "--app" => application = Some(next_value(args, &mut index, "--app")?.to_owned()),
            "--pin" => pin = true,
            "--copy" => copy_to_clipboard = true,
            "-h" | "--help" => return Ok(CliAction::Help),
            "-" if raw_text.is_none() => raw_text = Some("-".to_owned()),
            arg if arg.starts_with('-') => return Err(format!("unknown option '{arg}' for add")),
            positional if raw_text.is_none() => raw_text = Some(positional.to_owned()),
            extra => return Err(format!("unexpected argument '{extra}' for add")),
        }
        index += 1;
    }
    let text = match raw_text.as_deref() {
        Some("-") => {
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .map_err(|error| format!("failed to read stdin: {error}"))?;
            buffer
        }
        Some(value) => value.to_owned(),
        None => return Err("add requires <text> or '-' to read stdin".to_owned()),
    };
    Ok(CliAction::Control {
        request: ControlRequest::AddText(AddTextParams {
            text,
            application,
            pin,
            copy_to_clipboard,
        }),
        json,
        database,
    })
}

fn parse_settings(
    args: &[&str],
    mut database: Option<String>,
    json: &mut bool,
) -> Result<CliAction, String> {
    let Some((&action, rest)) = args.split_first() else {
        return Err("settings requires a subcommand: show | set <key> <value>".to_owned());
    };
    match action {
        "show" => parse_no_args(ControlRequest::SettingsGet, rest, database, *json),
        "set" => {
            let mut key = None;
            let mut raw_value = None;
            let mut index = 0;
            while index < rest.len() {
                match rest[index] {
                    "--json" => *json = true,
                    "--database" => {
                        database = Some(next_value(rest, &mut index, "--database")?.to_owned());
                    }
                    "-h" | "--help" => return Ok(CliAction::Help),
                    arg if arg.starts_with('-') && raw_value.is_some() => {
                        return Err(format!("unknown option '{arg}' for settings set"));
                    }
                    positional if key.is_none() => key = Some(positional.to_owned()),
                    positional if raw_value.is_none() => raw_value = Some(positional.to_owned()),
                    extra => return Err(format!("unexpected argument '{extra}' for settings set")),
                }
                index += 1;
            }
            let key = key.ok_or_else(|| "settings set requires <key> <value>".to_owned())?;
            let raw = raw_value.ok_or_else(|| "settings set requires <key> <value>".to_owned())?;
            let value = parse_cli_setting_value(&raw);
            Ok(CliAction::Control {
                request: ControlRequest::SettingsSet(UpdateSettingParams { key, value }),
                json: *json,
                database,
            })
        }
        "-h" | "--help" => Ok(CliAction::Help),
        other => Err(format!(
            "unknown settings subcommand '{other}'; expected 'show' or 'set'"
        )),
    }
}

fn parse_cli_setting_value(raw: &str) -> Value {
    if raw.eq_ignore_ascii_case("true") {
        Value::Bool(true)
    } else if raw.eq_ignore_ascii_case("false") {
        Value::Bool(false)
    } else if let Ok(number) = raw.parse::<i64>() {
        Value::Number(number.into())
    } else {
        Value::String(raw.to_owned())
    }
}

fn next_value<'a>(args: &[&'a str], index: &mut usize, flag: &str) -> Result<&'a str, String> {
    *index += 1;
    args.get(*index)
        .copied()
        .ok_or_else(|| format!("'{flag}' requires a value"))
}

fn print_human(request: &ControlRequest, response: &ControlResponse) {
    if !response.ok {
        if let Some(error) = &response.error {
            eprintln!("error [{}]: {}", error.code, error.message);
        } else {
            eprintln!("error: command failed");
        }
        return;
    }
    let Some(data) = &response.data else {
        return;
    };
    match request {
        ControlRequest::Status => {
            println!(
                "Manis Pocket {} ({})\nSource: {:?}\nDatabase: {} ({} bytes)\nItems: {} ({} pinned)\nSync: {} (device: {}, paired: {}, discovered: {})",
                data["version"].as_str().unwrap_or(""),
                data["platform"].as_str().unwrap_or(""),
                response.source,
                data["database_path"].as_str().unwrap_or(""),
                data["database_bytes"].as_i64().unwrap_or(0),
                data["item_count"].as_u64().unwrap_or(0),
                data["pinned_count"].as_u64().unwrap_or(0),
                if data["sync_enabled"].as_bool().unwrap_or(false) {
                    "enabled"
                } else {
                    "disabled"
                },
                data["device_name"].as_str().unwrap_or(""),
                data["paired_peer_count"].as_u64().unwrap_or(0),
                data["discovered_peer_count"].as_u64().unwrap_or(0),
            );
        }
        ControlRequest::List(_) | ControlRequest::Search(_) => {
            let items = data["items"].as_array();
            let total = data["total_matching"].as_u64().unwrap_or(0);
            if let Some(items) = items {
                for item in items {
                    let pin = item["pin"]
                        .as_str()
                        .map(|p| format!("[pin:{p}] "))
                        .unwrap_or_default();
                    let kind = item["kind"].as_str().unwrap_or("?");
                    let id = item["id"].as_str().unwrap_or("");
                    let preview = item["title_preview"].as_str().unwrap_or("");
                    println!("{id}  ({kind}) {pin}{preview}");
                }
                println!("-- {} / {} items --", items.len(), total);
            }
        }
        ControlRequest::Get(_) => {
            if let Some(text) = data["text"].as_str() {
                println!("{text}");
                if data["truncated"].as_bool().unwrap_or(false) {
                    eprintln!(
                        "[truncated; total {} bytes]",
                        data["text_total_bytes"].as_u64().unwrap_or(0)
                    );
                }
            } else if let Some(image) = data.get("image").filter(|v| !v.is_null()) {
                println!(
                    "image {}x{} ({} bytes, sha256:{})",
                    image["width"].as_u64().unwrap_or(0),
                    image["height"].as_u64().unwrap_or(0),
                    image["size_bytes"].as_u64().unwrap_or(0),
                    image["sha256"].as_str().unwrap_or(""),
                );
                if let Some(path) = image["exported_path"].as_str() {
                    println!("exported: {path}");
                }
            } else if let Ok(pretty) = serde_json::to_string_pretty(data) {
                println!("{pretty}");
            }
        }
        _ => {
            if let Ok(pretty) = serde_json::to_string_pretty(data) {
                println!("{pretty}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_agent_subcommands_from_legacy_gui_flags() {
        assert!(is_cli_invocation(&["status", "--json"]));
        assert!(is_cli_invocation(&["--database", "/tmp/db", "list"]));
        assert!(is_cli_invocation(&["mcp"]));
        assert!(is_cli_invocation(&["--help"]));
        assert!(is_cli_invocation(&["--version"]));

        // Legacy GUI / IPC flags keep flowing into main untouched.
        assert!(!is_cli_invocation(&["--toggle"]));
        assert!(!is_cli_invocation(&["--settings"]));
        assert!(!is_cli_invocation(&["--database", "/tmp/db"]));
        assert!(!is_cli_invocation(&["--copy", "abc"]));
        assert!(!is_cli_invocation(&["--pair", "12D3Koo"]));
    }

    #[test]
    fn parses_subcommands_and_options() {
        assert_eq!(
            parse_args(&["status", "--json", "--database", "/tmp/a.db"]),
            Ok(CliAction::Control {
                request: ControlRequest::Status,
                json: true,
                database: Some("/tmp/a.db".to_owned()),
            })
        );

        assert_eq!(
            parse_args(&[
                "search", "hello", "--mode", "fuzzy", "--kind", "text", "--limit", "5", "--json"
            ]),
            Ok(CliAction::Control {
                request: ControlRequest::Search(SearchParams {
                    query: "hello".to_owned(),
                    mode: Some("fuzzy".to_owned()),
                    kind: Some("text".to_owned()),
                    app: None,
                    limit: Some(5),
                }),
                json: true,
                database: None,
            })
        );

        assert_eq!(
            parse_args(&["settings", "set", "historySize", "1000", "--json"]),
            Ok(CliAction::Control {
                request: ControlRequest::SettingsSet(UpdateSettingParams {
                    key: "historySize".to_owned(),
                    value: Value::Number(1000.into()),
                }),
                json: true,
                database: None,
            })
        );

        assert_eq!(
            parse_args(&["--database", "/tmp/x.db", "mcp"]),
            Ok(CliAction::Mcp {
                database: Some("/tmp/x.db".to_owned()),
            })
        );

        assert!(parse_args(&["search"]).is_err());
        assert!(parse_args(&["settings", "bogus"]).is_err());
    }
}
