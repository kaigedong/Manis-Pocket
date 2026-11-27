//! Local agent-first Control API (live Unix domain socket + offline SQLite fallback).
//!
//! When the Manis Pocket desktop app is running, CLI commands and MCP tools talk
//! to `<data_dir>/agent.sock` (`0o600`, same-user only) so live sync state is
//! visible and mutations refresh the open UI immediately. When the desktop app
//! is offline, requests fall back cleanly to the local SQLite database and
//! `settings.json` store.

use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use manis_pocket_core::history::HistoryManager;
use manis_pocket_core::model::{ClipboardContent, ClipboardItem, SearchMode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::app_update;
use crate::ignore;
use crate::panel::copy_entry_to_clipboard;
use crate::peers::PairedPeer;
use crate::rows::{self, RowKind};
use crate::settings::Settings;
use crate::sync_runtime;

const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_WIRE_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_LIST_LIMIT: usize = 20;
const MAX_LIST_LIMIT: usize = 200;
const TITLE_PREVIEW_CHARS: usize = 200;
const DEFAULT_MAX_CHARS: usize = 32_768;
const MAX_GET_CHARS: usize = 262_144;

/// Where a Control API response was served from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseSource {
    LiveApp,
    OfflineStore,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ErrorPayload {
    pub(crate) code: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ControlResponse {
    pub(crate) ok: bool,
    pub(crate) source: ResponseSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<ErrorPayload>,
}

impl ControlResponse {
    pub(crate) fn ok(source: ResponseSource, data: impl Serialize) -> Self {
        match serde_json::to_value(data) {
            Ok(value) => Self {
                ok: true,
                source,
                data: Some(value),
                error: None,
            },
            Err(error) => Self::err(source, "serialization_failed", error.to_string()),
        }
    }

    pub(crate) fn err(
        source: ResponseSource,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            ok: false,
            source,
            data: None,
            error: Some(ErrorPayload {
                code: code.into(),
                message: message.into(),
            }),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ListParams {
    #[serde(default)]
    pub(crate) limit: Option<usize>,
    #[serde(default)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) app: Option<String>,
    #[serde(default)]
    pub(crate) pinned_only: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SearchParams {
    pub(crate) query: String,
    #[serde(default)]
    pub(crate) mode: Option<String>,
    #[serde(default)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) app: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct GetParams {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) max_chars: Option<usize>,
    #[serde(default)]
    pub(crate) export_image_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct CopyParams {
    pub(crate) id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AddTextParams {
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) application: Option<String>,
    #[serde(default)]
    pub(crate) pin: bool,
    #[serde(default)]
    pub(crate) copy_to_clipboard: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct SetPinParams {
    pub(crate) id: String,
    pub(crate) pinned: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct DeleteParams {
    pub(crate) id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct UpdateSettingParams {
    pub(crate) key: String,
    pub(crate) value: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum ControlRequest {
    Status,
    List(ListParams),
    Search(SearchParams),
    Get(GetParams),
    Copy(CopyParams),
    AddText(AddTextParams),
    SetPin(SetPinParams),
    Delete(DeleteParams),
    Peers,
    SettingsGet,
    SettingsSet(UpdateSettingParams),
}

impl ControlRequest {
    fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::AddText(_) | Self::SetPin(_) | Self::Delete(_) | Self::SettingsSet(_)
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct StatusData {
    pub(crate) version: String,
    pub(crate) platform: String,
    pub(crate) database_path: String,
    pub(crate) database_bytes: i64,
    pub(crate) item_count: usize,
    pub(crate) pinned_count: usize,
    pub(crate) sync_enabled: bool,
    pub(crate) device_name: String,
    pub(crate) listening_address: Option<String>,
    pub(crate) paired_peer_count: usize,
    pub(crate) discovered_peer_count: usize,
    pub(crate) pending_pairing_count: usize,
    pub(crate) last_sync_error: Option<String>,
    pub(crate) supports_self_install: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HistorySummary {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) title_preview: String,
    pub(crate) application: Option<String>,
    pub(crate) pin: Option<String>,
    pub(crate) number_of_copies: i32,
    pub(crate) first_copied_at: i64,
    pub(crate) last_copied_at: i64,
    pub(crate) sync_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) score: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HistoryListData {
    pub(crate) total_matching: usize,
    pub(crate) returned: usize,
    pub(crate) items: Vec<HistorySummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ImageMetadata {
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) size_bytes: usize,
    pub(crate) sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) exported_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ItemDetailData {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) application: Option<String>,
    pub(crate) pin: Option<String>,
    pub(crate) number_of_copies: i32,
    pub(crate) first_copied_at: i64,
    pub(crate) last_copied_at: i64,
    pub(crate) sync_source: Option<String>,
    pub(crate) content_types: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text_total_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) files: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) image: Option<ImageMetadata>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PairedPeerData {
    pub(crate) peer_id: String,
    pub(crate) display_name: String,
    pub(crate) is_online: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct DiscoveredPeerData {
    pub(crate) peer_id: String,
    pub(crate) display_name: String,
    pub(crate) is_connected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PendingPairingData {
    pub(crate) peer_id: String,
    pub(crate) display_name: String,
    pub(crate) requires_human_pin_confirmation: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PeersData {
    pub(crate) sync_enabled: bool,
    pub(crate) device_name: String,
    pub(crate) listening_address: Option<String>,
    pub(crate) paired: Vec<PairedPeerData>,
    pub(crate) discovered: Vec<DiscoveredPeerData>,
    pub(crate) pending_pairings: Vec<PendingPairingData>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SettingsData {
    pub(crate) history_size: i64,
    pub(crate) unlimited_history: bool,
    pub(crate) paste_by_default: bool,
    pub(crate) show_footer: bool,
    pub(crate) show_search: bool,
    pub(crate) show_title: bool,
    pub(crate) show_application_icons: bool,
    pub(crate) show_in_status_bar: bool,
    pub(crate) show_special_symbols: bool,
    pub(crate) search_mode: String,
    pub(crate) sort_by: String,
    pub(crate) popup_position: String,
    pub(crate) save_images: bool,
    pub(crate) save_text: bool,
    pub(crate) pins_position: String,
    pub(crate) sync_enabled: bool,
    pub(crate) sync_device_name: String,
    pub(crate) ignored_apps: Vec<String>,
    pub(crate) ignore_all_apps_except_listed: bool,
    pub(crate) ignore_regexp_count: usize,
}

/// Path of the same-user JSON Control API socket next to the history database.
pub(crate) fn socket_path(database: &str) -> PathBuf {
    let database = PathBuf::from(database);
    let directory = database
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&directory);
    directory.join("agent.sock")
}

/// Starts listening on `<data_dir>/agent.sock` (`0o600`) for live Agent CLI / MCP calls.
pub(crate) fn listen(
    database: String,
    history: Arc<HistoryManager>,
    on_mutate: Arc<dyn Fn() + Send + Sync>,
) -> bool {
    let path = socket_path(&database);
    let listener = match bind_private_socket(&path) {
        Some(listener) => listener,
        None => return false,
    };

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            handle_client(stream, &database, &history, &on_mutate);
        }
    });
    true
}

fn bind_private_socket(path: &Path) -> Option<UnixListener> {
    let listener = match UnixListener::bind(path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                return None;
            }
            let _ = std::fs::remove_file(path);
            UnixListener::bind(path).ok()?
        }
        Err(_) => return None,
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Some(listener)
}

fn handle_client(
    stream: UnixStream,
    database: &str,
    history: &HistoryManager,
    on_mutate: &Arc<dyn Fn() + Send + Sync>,
) {
    let _ = stream.set_read_timeout(Some(SOCKET_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SOCKET_TIMEOUT));

    let Ok(writer_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream.take(MAX_WIRE_BYTES as u64));
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
        return;
    }

    let response = match serde_json::from_str::<ControlRequest>(line.trim()) {
        Ok(request) => {
            let response =
                execute_with_manager(history, database, ResponseSource::LiveApp, &request);
            if response.ok && request.is_mutating() {
                on_mutate();
            }
            response
        }
        Err(error) => ControlResponse::err(
            ResponseSource::LiveApp,
            "invalid_request",
            format!("invalid control request: {error}"),
        ),
    };

    if let Ok(encoded) = serde_json::to_string(&response) {
        let mut writer = writer_stream;
        let _ = writer.write_all(encoded.as_bytes());
        let _ = writer.write_all(b"\n");
        let _ = writer.flush();
    }
}

fn send_request(database: &str, request: &ControlRequest) -> Option<ControlResponse> {
    let mut stream = UnixStream::connect(socket_path(database)).ok()?;
    let _ = stream.set_read_timeout(Some(SOCKET_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SOCKET_TIMEOUT));
    let encoded = serde_json::to_string(request).ok()?;
    stream.write_all(encoded.as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;
    stream.flush().ok()?;

    let mut line = String::new();
    let mut reader = BufReader::new(stream.take(MAX_WIRE_BYTES as u64));
    reader.read_line(&mut line).ok()?;
    serde_json::from_str(line.trim()).ok()
}

/// Executes a control request against the live desktop app when available,
/// falling back to the local SQLite database and `settings.json` when offline.
pub(crate) fn execute(database: &str, request: &ControlRequest) -> ControlResponse {
    if database != ":memory:"
        && let Some(response) = send_request(database, request)
    {
        return response;
    }

    let history = match HistoryManager::new(database.to_owned()) {
        Ok(history) => history,
        Err(error) => {
            return ControlResponse::err(
                ResponseSource::OfflineStore,
                "storage_unavailable",
                format!("could not open history database '{database}': {error}"),
            );
        }
    };
    execute_with_manager(&history, database, ResponseSource::OfflineStore, request)
}

pub(crate) fn execute_with_manager(
    history: &HistoryManager,
    database: &str,
    source: ResponseSource,
    request: &ControlRequest,
) -> ControlResponse {
    match request {
        ControlRequest::Status => handle_status(history, database, source),
        ControlRequest::List(params) => handle_list(history, database, source, params),
        ControlRequest::Search(params) => handle_search(history, database, source, params),
        ControlRequest::Get(params) => handle_get(history, source, params),
        ControlRequest::Copy(params) => handle_copy(history, source, params),
        ControlRequest::AddText(params) => handle_add_text(history, database, source, params),
        ControlRequest::SetPin(params) => handle_set_pin(history, source, params),
        ControlRequest::Delete(params) => handle_delete(history, source, params),
        ControlRequest::Peers => handle_peers(history, database, source),
        ControlRequest::SettingsGet => handle_settings_get(database, source),
        ControlRequest::SettingsSet(params) => handle_settings_set(database, source, params),
    }
}

fn handle_status(
    history: &HistoryManager,
    database: &str,
    source: ResponseSource,
) -> ControlResponse {
    let settings = Settings::load(database);
    let items = match history.load_metadata() {
        Ok(items) => items,
        Err(error) => return ControlResponse::err(source, "storage_error", error.to_string()),
    };
    let pinned_count = items.iter().filter(|item| item.pin.is_some()).count();
    let paired_peer_count = history
        .get_paired_peers()
        .iter()
        .filter_map(|line| PairedPeer::from_json(line))
        .count();
    let (listening_address, discovered_peer_count, pending_pairing_count, last_sync_error) =
        if source == ResponseSource::LiveApp {
            let sync = sync_runtime::status();
            (
                sync.listening,
                sync_runtime::discovered_peers().len(),
                sync_runtime::pending_pairings().len(),
                sync.last_error,
            )
        } else {
            (None, 0, 0, None)
        };

    ControlResponse::ok(
        source,
        StatusData {
            version: app_update::current_version().to_owned(),
            platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            database_path: database.to_owned(),
            database_bytes: history.storage_size_bytes(database.to_owned()),
            item_count: items.len(),
            pinned_count,
            sync_enabled: settings.sync_enabled,
            device_name: settings.sync_device_name,
            listening_address,
            paired_peer_count,
            discovered_peer_count,
            pending_pairing_count,
            last_sync_error,
            supports_self_install: app_update::supports_self_install(),
        },
    )
}

fn parse_kind_filter(kind: Option<&str>) -> Result<Option<RowKind>, String> {
    match kind.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some("text") => Ok(Some(RowKind::Text)),
        Some("image") => Ok(Some(RowKind::Image)),
        Some("file") => Ok(Some(RowKind::File)),
        Some("other") => Ok(Some(RowKind::Other)),
        Some(other) => Err(format!(
            "unknown kind '{other}'; expected one of: text, image, file, other"
        )),
    }
}

fn row_kind_name(kind: RowKind) -> &'static str {
    match kind {
        RowKind::Text => "text",
        RowKind::Image => "image",
        RowKind::File => "file",
        RowKind::Other => "other",
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> (String, bool) {
    let mut chars = text.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    let truncated = chars.next().is_some();
    (collected, truncated)
}

fn summarize_item(item: &ClipboardItem, score: Option<f64>) -> HistorySummary {
    let kind = rows::row_kind(item);
    let title = rows::row_title_with_symbols(item, false);
    let (mut preview, truncated) = truncate_chars(&title, TITLE_PREVIEW_CHARS);
    if truncated {
        preview.push('…');
    }
    HistorySummary {
        id: item.id.clone(),
        kind: row_kind_name(kind).to_owned(),
        title_preview: preview,
        application: item.application.clone(),
        pin: item.pin.clone(),
        number_of_copies: item.number_of_copies,
        first_copied_at: item.first_copied_at,
        last_copied_at: item.last_copied_at,
        sync_source: item.sync_source.clone(),
        score,
    }
}

fn matches_filters(item: &ClipboardItem, kind: Option<RowKind>, app_needle: Option<&str>) -> bool {
    if let Some(expected_kind) = kind
        && rows::row_kind(item) != expected_kind
    {
        return false;
    }
    if let Some(needle) = app_needle {
        let matched = item
            .application
            .as_deref()
            .is_some_and(|app| app.to_ascii_lowercase().contains(needle));
        if !matched {
            return false;
        }
    }
    true
}

fn handle_list(
    history: &HistoryManager,
    database: &str,
    source: ResponseSource,
    params: &ListParams,
) -> ControlResponse {
    let kind = match parse_kind_filter(params.kind.as_deref()) {
        Ok(kind) => kind,
        Err(message) => return ControlResponse::err(source, "invalid_arg", message),
    };
    let app_needle = params
        .app
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let limit = params
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);

    let settings = Settings::load(database);
    let items = match history.load_metadata() {
        Ok(items) => items,
        Err(error) => return ControlResponse::err(source, "storage_error", error.to_string()),
    };
    let arranged = rows::arrange(items, &settings.pins_position, &settings.sort_by);
    let filtered: Vec<ClipboardItem> = arranged
        .items
        .into_iter()
        .filter(|item| !params.pinned_only || item.pin.is_some())
        .filter(|item| matches_filters(item, kind, app_needle.as_deref()))
        .collect();
    let total_matching = filtered.len();
    let summaries: Vec<HistorySummary> = filtered
        .iter()
        .take(limit)
        .map(|item| summarize_item(item, None))
        .collect();

    ControlResponse::ok(
        source,
        HistoryListData {
            total_matching,
            returned: summaries.len(),
            items: summaries,
        },
    )
}

fn parse_search_mode(value: &str) -> Result<SearchMode, String> {
    match value {
        "exact" => Ok(SearchMode::Exact),
        "fuzzy" => Ok(SearchMode::Fuzzy),
        "regexp" => Ok(SearchMode::Regexp),
        "mixed" => Ok(SearchMode::Mixed),
        other => Err(format!(
            "unknown search mode '{other}'; expected one of: exact, fuzzy, regexp, mixed"
        )),
    }
}

fn handle_search(
    history: &HistoryManager,
    database: &str,
    source: ResponseSource,
    params: &SearchParams,
) -> ControlResponse {
    let kind = match parse_kind_filter(params.kind.as_deref()) {
        Ok(kind) => kind,
        Err(message) => return ControlResponse::err(source, "invalid_arg", message),
    };
    let settings = Settings::load(database);
    let mode_name = params.mode.as_deref().unwrap_or(&settings.search_mode);
    let mode = match parse_search_mode(mode_name) {
        Ok(mode) => mode,
        Err(message) => return ControlResponse::err(source, "invalid_arg", message),
    };
    let app_needle = params
        .app
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let limit = params
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);

    let items = match history.load_metadata() {
        Ok(items) => items,
        Err(error) => return ControlResponse::err(source, "storage_error", error.to_string()),
    };
    let pool: Vec<ClipboardItem> = items
        .into_iter()
        .filter(|item| matches_filters(item, kind, app_needle.as_deref()))
        .collect();
    let results = history.search(&params.query, pool, mode);
    let total_matching = results.len();
    let summaries: Vec<HistorySummary> = results
        .iter()
        .take(limit)
        .map(|result| summarize_item(&result.item, result.score))
        .collect();

    ControlResponse::ok(
        source,
        HistoryListData {
            total_matching,
            returned: summaries.len(),
            items: summaries,
        },
    )
}

fn find_item_metadata(history: &HistoryManager, id: &str) -> Result<Option<ClipboardItem>, String> {
    let items = history.load_metadata().map_err(|error| error.to_string())?;
    Ok(items.into_iter().find(|item| item.id == id))
}

fn handle_get(
    history: &HistoryManager,
    source: ResponseSource,
    params: &GetParams,
) -> ControlResponse {
    let item = match find_item_metadata(history, &params.id) {
        Ok(Some(item)) => item,
        Ok(None) => {
            return ControlResponse::err(
                source,
                "item_not_found",
                format!("clipboard item '{}' not found", params.id),
            );
        }
        Err(message) => return ControlResponse::err(source, "storage_error", message),
    };

    let contents = match history.contents(params.id.clone()) {
        Ok(contents) => contents,
        Err(error) => return ControlResponse::err(source, "storage_error", error.to_string()),
    };
    let max_chars = params
        .max_chars
        .unwrap_or(DEFAULT_MAX_CHARS)
        .clamp(1, MAX_GET_CHARS);
    let kind = rows::row_kind(&item);
    let content_types: Vec<String> = contents.iter().map(|c| c.content_type.clone()).collect();

    let text_bytes = contents
        .iter()
        .find(|content| {
            matches!(
                content.content_type.as_str(),
                "public.utf8-plain-text" | "text/plain" | "text/plain;charset=utf-8"
            )
        })
        .and_then(|content| content.value.as_ref());
    let (text, text_total_bytes, truncated) = match text_bytes {
        Some(bytes) => {
            let full = String::from_utf8_lossy(bytes);
            let (clipped, was_truncated) = truncate_chars(&full, max_chars);
            (Some(clipped), Some(bytes.len()), Some(was_truncated))
        }
        None => (None, None, None),
    };

    let files = contents
        .iter()
        .find(|content| content.content_type == "public.file-url")
        .and_then(|content| content.value.as_ref())
        .map(|bytes| {
            String::from_utf8_lossy(bytes)
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        });

    let image_bytes = contents
        .iter()
        .find(|content| {
            matches!(
                content.content_type.as_str(),
                "public.png" | "image/png" | "public.tiff"
            )
        })
        .and_then(|content| content.value.as_ref());

    let image = match image_bytes {
        Some(bytes) => {
            let dims = image::ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .ok()
                .and_then(|reader| reader.into_dimensions().ok());
            let exported_path = if let Some(dest) = params
                .export_image_path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
            {
                if let Err(error) = std::fs::write(dest, bytes) {
                    return ControlResponse::err(
                        source,
                        "export_failed",
                        format!("failed to export image to '{dest}': {error}"),
                    );
                }
                Some(dest.to_owned())
            } else {
                None
            };
            Some(ImageMetadata {
                width: dims.map(|(w, _)| w),
                height: dims.map(|(_, h)| h),
                size_bytes: bytes.len(),
                sha256: hex::encode(Sha256::digest(bytes)),
                exported_path,
            })
        }
        None => {
            if params
                .export_image_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
            {
                return ControlResponse::err(
                    source,
                    "not_an_image",
                    format!("clipboard item '{}' does not contain image data", params.id),
                );
            }
            None
        }
    };

    ControlResponse::ok(
        source,
        ItemDetailData {
            id: item.id,
            kind: row_kind_name(kind).to_owned(),
            title: item.title,
            application: item.application,
            pin: item.pin,
            number_of_copies: item.number_of_copies,
            first_copied_at: item.first_copied_at,
            last_copied_at: item.last_copied_at,
            sync_source: item.sync_source,
            content_types,
            text,
            text_total_bytes,
            truncated,
            files,
            image,
        },
    )
}

fn handle_copy(
    history: &HistoryManager,
    source: ResponseSource,
    params: &CopyParams,
) -> ControlResponse {
    match find_item_metadata(history, &params.id) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return ControlResponse::err(
                source,
                "item_not_found",
                format!("clipboard item '{}' not found", params.id),
            );
        }
        Err(message) => return ControlResponse::err(source, "storage_error", message),
    }

    if copy_entry_to_clipboard(history, &params.id) {
        ControlResponse::ok(source, json!({ "copied": true, "id": params.id }))
    } else {
        ControlResponse::err(
            source,
            "clipboard_write_failed",
            format!("could not write item '{}' to system clipboard", params.id),
        )
    }
}

fn handle_add_text(
    history: &HistoryManager,
    database: &str,
    source: ResponseSource,
    params: &AddTextParams,
) -> ControlResponse {
    if params.text.is_empty() {
        return ControlResponse::err(source, "invalid_arg", "text must not be empty");
    }
    let settings = Settings::load(database);
    if ignore::should_ignore(&settings, params.application.as_deref(), &params.text) {
        return ControlResponse::err(
            source,
            "ignored_by_policy",
            "text matched Manis Pocket ignore rules and was not stored",
        );
    }

    let now = Utc::now().timestamp_millis();
    let bytes = params.text.as_bytes().to_vec();
    let title: String = params.text.chars().take(120).collect();
    let item = ClipboardItem {
        id: uuid::Uuid::new_v4().to_string(),
        application: params.application.clone(),
        first_copied_at: now,
        last_copied_at: now,
        number_of_copies: 1,
        pin: None,
        title,
        contents: vec![ClipboardContent {
            content_type: "public.utf8-plain-text".to_owned(),
            value: Some(bytes.clone()),
        }],
        sync_timestamp: now,
        sync_source: None,
        sync_deleted: false,
    };

    let mut stored = match history.add(
        item,
        settings.history_size as i32,
        settings.unlimited_history,
    ) {
        Ok(stored) => stored,
        Err(error) => return ControlResponse::err(source, "storage_error", error.to_string()),
    };

    if params.pin && stored.pin.is_none() {
        let used: Vec<String> = history
            .load_metadata()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| entry.pin)
            .collect();
        let available = rows::available_pins(&used);
        if let Ok(pinned) = history.toggle_pin(stored.id.clone(), available) {
            stored = pinned;
        }
    }

    let copied = if params.copy_to_clipboard {
        crate::clipboard::write_text(&params.text)
    } else {
        false
    };

    if source == ResponseSource::LiveApp {
        history.sync_observe_local_clipboard(Some(ClipboardContent {
            content_type: "public.utf8-plain-text".to_owned(),
            value: Some(bytes),
        }));
    }

    ControlResponse::ok(
        source,
        json!({
            "item": summarize_item(&stored, None),
            "copied_to_clipboard": copied,
        }),
    )
}

fn handle_set_pin(
    history: &HistoryManager,
    source: ResponseSource,
    params: &SetPinParams,
) -> ControlResponse {
    let items = match history.load_metadata() {
        Ok(items) => items,
        Err(error) => return ControlResponse::err(source, "storage_error", error.to_string()),
    };
    let Some(existing) = items.iter().find(|item| item.id == params.id) else {
        return ControlResponse::err(
            source,
            "item_not_found",
            format!("clipboard item '{}' not found", params.id),
        );
    };

    // Idempotent: setting the same pin state twice is a clean no-op.
    let currently_pinned = existing.pin.is_some();
    let updated = if currently_pinned == params.pinned {
        existing.clone()
    } else if params.pinned {
        let used: Vec<String> = items.iter().filter_map(|entry| entry.pin.clone()).collect();
        let available = rows::available_pins(&used);
        if available.is_empty() {
            return ControlResponse::err(
                source,
                "no_available_pins",
                "all pin shortcut letters are already in use",
            );
        }
        match history.toggle_pin(params.id.clone(), available) {
            Ok(updated) => updated,
            Err(error) => return ControlResponse::err(source, "storage_error", error.to_string()),
        }
    } else {
        match history.toggle_pin(params.id.clone(), Vec::new()) {
            Ok(updated) => updated,
            Err(error) => return ControlResponse::err(source, "storage_error", error.to_string()),
        }
    };

    ControlResponse::ok(
        source,
        json!({
            "id": updated.id,
            "pinned": updated.pin.is_some(),
            "pin": updated.pin,
        }),
    )
}

fn handle_delete(
    history: &HistoryManager,
    source: ResponseSource,
    params: &DeleteParams,
) -> ControlResponse {
    match find_item_metadata(history, &params.id) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return ControlResponse::err(
                source,
                "item_not_found",
                format!("clipboard item '{}' not found", params.id),
            );
        }
        Err(message) => return ControlResponse::err(source, "storage_error", message),
    }

    match history.delete(params.id.clone()) {
        Ok(()) => ControlResponse::ok(source, json!({ "deleted": true, "id": params.id })),
        Err(error) => ControlResponse::err(source, "storage_error", error.to_string()),
    }
}

fn handle_peers(
    history: &HistoryManager,
    database: &str,
    source: ResponseSource,
) -> ControlResponse {
    let settings = Settings::load(database);
    let paired: Vec<PairedPeerData> = history
        .get_paired_peers()
        .iter()
        .filter_map(|line| PairedPeer::from_json(line))
        .map(|peer| PairedPeerData {
            peer_id: peer.peer_id,
            display_name: peer.display_name,
            is_online: peer.is_online,
        })
        .collect();

    let (listening_address, discovered, pending_pairings) = if source == ResponseSource::LiveApp {
        let sync = sync_runtime::status();
        let discovered = sync_runtime::discovered_peers()
            .into_iter()
            .map(|peer| DiscoveredPeerData {
                peer_id: peer.peer_id,
                display_name: peer.display_name,
                is_connected: peer.is_connected,
            })
            .collect();
        let pending = sync_runtime::pending_pairings()
            .into_iter()
            .map(|pending| PendingPairingData {
                peer_id: pending.peer_id,
                display_name: pending.display_name,
                requires_human_pin_confirmation: true,
            })
            .collect();
        (sync.listening, discovered, pending)
    } else {
        (None, Vec::new(), Vec::new())
    };

    ControlResponse::ok(
        source,
        PeersData {
            sync_enabled: settings.sync_enabled,
            device_name: settings.sync_device_name,
            listening_address,
            paired,
            discovered,
            pending_pairings,
        },
    )
}

fn settings_to_data(settings: &Settings) -> SettingsData {
    SettingsData {
        history_size: settings.history_size,
        unlimited_history: settings.unlimited_history,
        paste_by_default: settings.paste_by_default,
        show_footer: settings.show_footer,
        show_search: settings.show_search,
        show_title: settings.show_title,
        show_application_icons: settings.show_application_icons,
        show_in_status_bar: settings.show_in_status_bar,
        show_special_symbols: settings.show_special_symbols,
        search_mode: settings.search_mode.clone(),
        sort_by: settings.sort_by.clone(),
        popup_position: settings.popup_position.clone(),
        save_images: settings.save_images,
        save_text: settings.save_text,
        pins_position: settings.pins_position.clone(),
        sync_enabled: settings.sync_enabled,
        sync_device_name: settings.sync_device_name.clone(),
        ignored_apps: settings.ignored_apps.clone(),
        ignore_all_apps_except_listed: settings.ignore_all_apps_except_listed,
        ignore_regexp_count: settings.ignore_regexp.len(),
    }
}

fn handle_settings_get(database: &str, source: ResponseSource) -> ControlResponse {
    let settings = Settings::load(database);
    ControlResponse::ok(source, settings_to_data(&settings))
}

fn expect_bool(key: &str, value: &Value) -> Result<bool, String> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        Value::String(text) if text.eq_ignore_ascii_case("true") || text == "1" => Ok(true),
        Value::String(text) if text.eq_ignore_ascii_case("false") || text == "0" => Ok(false),
        _ => Err(format!("'{key}' expects a boolean (true/false)")),
    }
}

fn expect_string<'a>(key: &str, value: &'a Value) -> Result<&'a str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("'{key}' expects a string value"))
}

fn expect_i64(key: &str, value: &Value) -> Result<i64, String> {
    match value {
        Value::Number(num) => num
            .as_i64()
            .ok_or_else(|| format!("'{key}' expects an integer")),
        Value::String(text) => text
            .trim()
            .parse::<i64>()
            .map_err(|_| format!("'{key}' expects an integer")),
        _ => Err(format!("'{key}' expects an integer")),
    }
}

fn handle_settings_set(
    database: &str,
    source: ResponseSource,
    params: &UpdateSettingParams,
) -> ControlResponse {
    let mut settings = Settings::load(database);
    let key = params.key.as_str();
    let result: Result<(), String> = match key {
        "searchMode" => expect_string(key, &params.value).and_then(|value| {
            parse_search_mode(value)?;
            settings.search_mode = value.to_owned();
            Ok(())
        }),
        "sortBy" => expect_string(key, &params.value).and_then(|value| match value {
            "lastCopiedAt" | "firstCopiedAt" | "numberOfCopies" => {
                settings.sort_by = value.to_owned();
                Ok(())
            }
            _ => Err(
                "'sortBy' must be one of: lastCopiedAt, firstCopiedAt, numberOfCopies".to_owned(),
            ),
        }),
        "pinsPosition" => expect_string(key, &params.value).and_then(|value| match value {
            "top" | "bottom" => {
                settings.pins_position = value.to_owned();
                Ok(())
            }
            _ => Err("'pinsPosition' must be one of: top, bottom".to_owned()),
        }),
        "popupPosition" => expect_string(key, &params.value).and_then(|value| match value {
            "statusItem" | "center" => {
                settings.popup_position = value.to_owned();
                Ok(())
            }
            _ => Err("'popupPosition' must be one of: statusItem, center".to_owned()),
        }),
        "historySize" => expect_i64(key, &params.value).and_then(|value| {
            if (1..=9999).contains(&value) {
                settings.history_size = value;
                Ok(())
            } else {
                Err("'historySize' must be between 1 and 9999".to_owned())
            }
        }),
        "unlimitedHistory" => expect_bool(key, &params.value).map(|v| {
            settings.unlimited_history = v;
        }),
        "pasteByDefault" => expect_bool(key, &params.value).map(|v| {
            settings.paste_by_default = v;
        }),
        "saveImages" => expect_bool(key, &params.value).map(|v| {
            settings.save_images = v;
        }),
        "saveText" => expect_bool(key, &params.value).map(|v| {
            settings.save_text = v;
        }),
        "showFooter" => expect_bool(key, &params.value).map(|v| {
            settings.show_footer = v;
        }),
        "showSearch" => expect_bool(key, &params.value).map(|v| {
            settings.show_search = v;
        }),
        "showTitle" => expect_bool(key, &params.value).map(|v| {
            settings.show_title = v;
        }),
        "syncEnabled" => expect_bool(key, &params.value).map(|v| {
            settings.sync_enabled = v;
        }),
        other => Err(format!(
            "unsupported setting '{other}'; supported keys: searchMode, sortBy, pinsPosition, popupPosition, historySize, unlimitedHistory, pasteByDefault, saveImages, saveText, showFooter, showSearch, showTitle, syncEnabled"
        )),
    };

    if let Err(message) = result {
        return ControlResponse::err(source, "invalid_arg", message);
    }

    settings.save(database);
    ControlResponse::ok(source, settings_to_data(&settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch_db(name: &str) -> (PathBuf, String) {
        // Keep the path short so macOS Unix domain socket paths stay well under
        // the 104-byte `sun_path` limit even under `/var/folders/.../T`.
        let dir = std::env::temp_dir().join(format!("mp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let db = dir.join("manispocket.db").to_string_lossy().into_owned();
        (dir, db)
    }

    #[test]
    fn offline_crud_search_pin_and_settings_round_trip() {
        let (dir, db) = scratch_db("crud");

        let added = execute(
            &db,
            &ControlRequest::AddText(AddTextParams {
                text: "agent-first clipboard payload".to_owned(),
                application: Some("com.apple.Terminal".to_owned()),
                pin: false,
                copy_to_clipboard: false,
            }),
        );
        assert!(added.ok, "{added:?}");
        assert_eq!(added.source, ResponseSource::OfflineStore);
        let id = added
            .data
            .as_ref()
            .and_then(|d| d["item"]["id"].as_str())
            .expect("item id")
            .to_owned();

        // List & search find the item cleanly.
        let listed = execute(&db, &ControlRequest::List(ListParams::default()));
        assert!(listed.ok);
        assert_eq!(listed.data.as_ref().unwrap()["returned"], 1);

        let searched = execute(
            &db,
            &ControlRequest::Search(SearchParams {
                query: "clipboard".to_owned(),
                mode: Some("exact".to_owned()),
                kind: Some("text".to_owned()),
                app: Some("terminal".to_owned()),
                limit: Some(10),
            }),
        );
        assert!(searched.ok);
        assert_eq!(searched.data.as_ref().unwrap()["returned"], 1);

        // Get supports bounded character truncation.
        let got = execute(
            &db,
            &ControlRequest::Get(GetParams {
                id: id.clone(),
                max_chars: Some(11),
                export_image_path: None,
            }),
        );
        assert!(got.ok);
        assert_eq!(got.data.as_ref().unwrap()["text"], "agent-first");
        assert_eq!(got.data.as_ref().unwrap()["truncated"], true);

        // Pin & unpin are idempotent.
        let pin1 = execute(
            &db,
            &ControlRequest::SetPin(SetPinParams {
                id: id.clone(),
                pinned: true,
            }),
        );
        assert!(pin1.ok);
        let pin_letter = pin1.data.as_ref().unwrap()["pin"]
            .as_str()
            .unwrap()
            .to_owned();
        let pin2 = execute(
            &db,
            &ControlRequest::SetPin(SetPinParams {
                id: id.clone(),
                pinned: true,
            }),
        );
        assert_eq!(
            pin2.data.as_ref().unwrap()["pin"].as_str(),
            Some(pin_letter.as_str())
        );

        let unpin = execute(
            &db,
            &ControlRequest::SetPin(SetPinParams {
                id: id.clone(),
                pinned: false,
            }),
        );
        assert_eq!(unpin.data.as_ref().unwrap()["pinned"], false);

        // Settings update validates allowed values and persists.
        let bad = execute(
            &db,
            &ControlRequest::SettingsSet(UpdateSettingParams {
                key: "searchMode".to_owned(),
                value: json!("bogus"),
            }),
        );
        assert!(!bad.ok);
        assert_eq!(bad.error.as_ref().unwrap().code, "invalid_arg");

        let ok_setting = execute(
            &db,
            &ControlRequest::SettingsSet(UpdateSettingParams {
                key: "searchMode".to_owned(),
                value: json!("fuzzy"),
            }),
        );
        assert!(ok_setting.ok);
        assert_eq!(ok_setting.data.as_ref().unwrap()["search_mode"], "fuzzy");

        // Delete removes item and a second delete returns item_not_found.
        let del = execute(
            &db,
            &ControlRequest::Delete(DeleteParams { id: id.clone() }),
        );
        assert!(del.ok);
        let del_again = execute(&db, &ControlRequest::Delete(DeleteParams { id }));
        assert!(!del_again.ok);
        assert_eq!(del_again.error.as_ref().unwrap().code, "item_not_found");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_text_honors_ignore_regexp_rules() {
        let (dir, db) = scratch_db("ignore");
        let settings = Settings {
            ignore_regexp: vec!["secret-[0-9]+".to_owned()],
            ..Settings::default()
        };
        settings.save(&db);

        let blocked = execute(
            &db,
            &ControlRequest::AddText(AddTextParams {
                text: "my secret-12345 token".to_owned(),
                application: None,
                pin: false,
                copy_to_clipboard: false,
            }),
        );
        assert!(!blocked.ok);
        assert_eq!(blocked.error.as_ref().unwrap().code, "ignored_by_policy");

        let shown = execute(&db, &ControlRequest::SettingsGet);
        assert!(shown.ok);
        assert_eq!(shown.data.as_ref().unwrap()["ignore_regexp_count"], 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_socket_serves_requests_with_private_permissions_and_notifies_mutations() {
        let (dir, db) = scratch_db("live");
        let history = Arc::new(HistoryManager::new(db.clone()).expect("open db"));
        let mutations = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&mutations);
        assert!(listen(
            db.clone(),
            history,
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            })
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(socket_path(&db))
                .expect("socket metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        let status = execute(&db, &ControlRequest::Status);
        assert!(status.ok);
        assert_eq!(status.source, ResponseSource::LiveApp);
        assert_eq!(mutations.load(Ordering::SeqCst), 0);

        let added = execute(
            &db,
            &ControlRequest::AddText(AddTextParams {
                text: "live socket item".to_owned(),
                application: None,
                pin: true,
                copy_to_clipboard: false,
            }),
        );
        assert!(added.ok);
        assert_eq!(added.source, ResponseSource::LiveApp);
        assert_eq!(mutations.load(Ordering::SeqCst), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
