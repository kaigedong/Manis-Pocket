use std::ffi::{CStr, CString};
use std::io::Write;
use std::os::raw::c_char;
use std::ptr;
use std::sync::Arc;

use crate::error::ErrorCode;
use crate::network::NetworkManager;
use crate::state::{SharedState, SyncCommand, SyncState};

pub type ManisPocketSync = Arc<std::sync::Mutex<SyncState>>;

struct StderrLogger;
impl log::Log for StderrLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        let _ = writeln!(
            std::io::stderr(),
            "[manis-pocket-sync] {} - {}",
            record.level(),
            record.args()
        );
    }
    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}
static LOGGER: StderrLogger = StderrLogger;

fn ensure_logger() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Debug);
}

fn c_str_to_string(s: *const c_char) -> Option<String> {
    if s.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(s).to_str().ok().map(String::from) }
}

fn with_state<F, R>(sync: *mut ManisPocketSync, f: F) -> R
where
    F: FnOnce(SharedState) -> R,
{
    let state: SharedState = unsafe { Arc::from_raw(sync as *const std::sync::Mutex<SyncState>) };
    let result = f(state.clone());
    std::mem::forget(state);
    result
}

// ── Lifecycle ─────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_create(
    device_name: *const c_char,
    device_id: *const c_char,
) -> *mut ManisPocketSync {
    let name = c_str_to_string(device_name).unwrap_or_default();
    let id = c_str_to_string(device_id).unwrap_or_default();
    ensure_logger();
    log::info!("manis_pocket_sync_create: name={}, id={}", name, id);

    match SyncState::new(&name, &id) {
        Ok(state) => Arc::into_raw(Arc::new(std::sync::Mutex::new(state))) as *mut ManisPocketSync,
        Err(_) => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_destroy(sync: *mut ManisPocketSync) {
    if !sync.is_null() {
        unsafe {
            let _ = Arc::from_raw(sync);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_start(sync: *mut ManisPocketSync) -> i32 {
    if sync.is_null() {
        return ErrorCode::InvalidArg as i32;
    }

    with_state(sync, |shared| {
        let local_key = libp2p::identity::Keypair::generate_ed25519();
        let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
        {
            let mut guard = shared.lock().unwrap();
            guard.command_tx = command_tx;
        }

        let mut manager = match NetworkManager::new(command_rx, shared.clone(), local_key) {
            Ok(m) => m,
            Err(e) => return e as i32,
        };

        let rt = {
            let guard = shared.lock().unwrap();
            guard.runtime.handle().clone()
        };

        std::thread::spawn(move || {
            rt.block_on(async move { manager.run().await });
        });

        log::info!("manis_pocket_sync_start: started successfully");
        ErrorCode::Ok as i32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_stop(sync: *mut ManisPocketSync) -> i32 {
    if sync.is_null() {
        return ErrorCode::InvalidArg as i32;
    }
    with_state(sync, |state| {
        let guard = state.lock().unwrap();
        let _ = guard.command_tx.send(SyncCommand::Shutdown);
        ErrorCode::Ok as i32
    })
}

// ── Single unified event callback ─────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_on_event(
    sync: *mut ManisPocketSync,
    cb: Option<extern "C" fn(event_json: *const c_char)>,
) {
    if sync.is_null() || cb.is_none() {
        return;
    }
    let cb = cb.unwrap();
    with_state(sync, |state| {
        let mut guard = state.lock().unwrap();
        *guard.on_event.lock() = Some(Box::new(move |json: &str| {
            let c_json = CString::new(json).unwrap_or_default();
            cb(c_json.as_ptr());
        }));
    });
}

// ── Discovery ─────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_start_discovery(sync: *mut ManisPocketSync) -> i32 {
    send_command(sync, SyncCommand::StartDiscovery)
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_stop_discovery(sync: *mut ManisPocketSync) -> i32 {
    send_command(sync, SyncCommand::StopDiscovery)
}

// ── Peer management ───────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_add_peer_address(
    sync: *mut ManisPocketSync,
    _peer_id: *const c_char,
    address: *const c_char,
) -> i32 {
    let addr = match c_str_to_string(address) {
        Some(s) => s,
        None => return ErrorCode::InvalidArg as i32,
    };
    send_command(sync, SyncCommand::AddPeerAddress { address: addr })
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_request_pairing(
    sync: *mut ManisPocketSync,
    peer_id: *const c_char,
) -> i32 {
    let pid = match c_str_to_string(peer_id) {
        Some(s) => s,
        None => return ErrorCode::InvalidArg as i32,
    };
    send_command(sync, SyncCommand::RequestPairing { peer_id: pid })
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_accept_pairing(
    sync: *mut ManisPocketSync,
    peer_id: *const c_char,
    pin: *const c_char,
) -> i32 {
    let pid = match c_str_to_string(peer_id) {
        Some(s) => s,
        None => return ErrorCode::InvalidArg as i32,
    };
    let p = match c_str_to_string(pin) {
        Some(s) => s,
        None => return ErrorCode::InvalidArg as i32,
    };
    send_command(
        sync,
        SyncCommand::AcceptPairing {
            peer_id: pid,
            pin: p,
        },
    )
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_reject_pairing(
    sync: *mut ManisPocketSync,
    peer_id: *const c_char,
) -> i32 {
    let pid = match c_str_to_string(peer_id) {
        Some(s) => s,
        None => return ErrorCode::InvalidArg as i32,
    };
    send_command(sync, SyncCommand::RejectPairing { peer_id: pid })
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_unpair(
    sync: *mut ManisPocketSync,
    peer_id: *const c_char,
) -> i32 {
    let pid = match c_str_to_string(peer_id) {
        Some(s) => s,
        None => return ErrorCode::InvalidArg as i32,
    };
    send_command(sync, SyncCommand::Unpair { peer_id: pid })
}

// ── Broadcast ─────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_broadcast_item(
    sync: *mut ManisPocketSync,
    item_json: *const c_char,
) -> i32 {
    let json = match c_str_to_string(item_json) {
        Some(s) => s,
        None => return ErrorCode::InvalidArg as i32,
    };
    send_command(sync, SyncCommand::BroadcastItem { item_json: json })
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_broadcast_deletion(
    sync: *mut ManisPocketSync,
    item_id: *const c_char,
) -> i32 {
    let id = match c_str_to_string(item_id) {
        Some(s) => s,
        None => return ErrorCode::InvalidArg as i32,
    };
    send_command(sync, SyncCommand::BroadcastDeletion { item_id: id })
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_broadcast_update(
    sync: *mut ManisPocketSync,
    item_json: *const c_char,
) -> i32 {
    let json = match c_str_to_string(item_json) {
        Some(s) => s,
        None => return ErrorCode::InvalidArg as i32,
    };
    send_command(sync, SyncCommand::BroadcastUpdate { item_json: json })
}

// ── Helpers ───────────────────────────────────────────────────────

fn send_command(sync: *mut ManisPocketSync, cmd: SyncCommand) -> i32 {
    if sync.is_null() {
        return ErrorCode::InvalidArg as i32;
    }
    log::info!("send_command: {:?}", cmd);
    with_state(sync, |state| {
        let guard = state.lock().unwrap();
        match guard.command_tx.send(cmd) {
            Ok(()) => ErrorCode::Ok as i32,
            Err(_) => ErrorCode::NotRunning as i32,
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_get_paired_peers(sync: *mut ManisPocketSync) -> *mut c_char {
    if sync.is_null() {
        return ptr::null_mut();
    }
    CString::new("[]").unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            let _ = CString::from_raw(s);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_is_running(sync: *mut ManisPocketSync) -> bool {
    !sync.is_null()
}

#[unsafe(no_mangle)]
pub extern "C" fn manis_pocket_sync_get_status(sync: *mut ManisPocketSync) -> *mut c_char {
    if sync.is_null() {
        return ptr::null_mut();
    }
    CString::new(r#"{"running":false}"#).unwrap().into_raw()
}

// ── Keep old callback symbols as no-ops for backward compat ───────

macro_rules! compat_callback {
    ($name:ident) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(_sync: *mut ManisPocketSync, _cb: Option<extern "C" fn()>) {}
    };
}

compat_callback!(manis_pocket_sync_on_peer_discovered);
compat_callback!(manis_pocket_sync_on_peer_lost);
compat_callback!(manis_pocket_sync_on_pairing_request);
compat_callback!(manis_pocket_sync_on_pairing_complete);
compat_callback!(manis_pocket_sync_on_sync_item_received);
compat_callback!(manis_pocket_sync_on_sync_item_deleted);
compat_callback!(manis_pocket_sync_on_sync_item_updated);
compat_callback!(manis_pocket_sync_on_error);
