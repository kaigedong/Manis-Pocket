//! Single-instance control socket.
//!
//! Launching the app again should bring the panel back rather than start a
//! second copy, and a compositor needs a command it can bind to a key. Both are
//! the same mechanism: a Unix socket the running instance listens on.
//!
//! This also gives Linux a working way to open the panel. `global-hotkey` grabs
//! keys through X11, which only receives them while an X11 client is focused, so
//! on a Wayland desktop the shortcut never fires even though registering it
//! reports success. Binding `manis-pocket --toggle` in the compositor works.

use futures::channel::mpsc::UnboundedSender;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

/// What a second launch can ask the running instance to do.
///
/// Pairing lives here as well as in the UI so a headless setup can be scripted —
/// which is also how the two machines are paired in testing.
#[derive(Clone, Debug)]
pub enum IpcCommand {
    Toggle,
    Pair(String),
    Accept { peer_id: String, pin: String },
    Connect(String),
}

/// The control socket lives next to the history database, so each profile gets
/// its own instance instead of fighting over a single global one.
pub fn socket_path(database: &str) -> PathBuf {
    let database = PathBuf::from(database);
    let directory = database
        .parent()
        .map(|parent| parent.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&directory);
    directory.join("control.sock")
}

/// Sends one command to the running instance. Returns false when nothing is
/// listening, which means the caller should start normally.
pub fn send(database: &str, line: &str) -> bool {
    let Ok(mut stream) = UnixStream::connect(socket_path(database)) else {
        return false;
    };
    stream.write_all(format!("{line}\n").as_bytes()).is_ok()
}

/// Asks the running instance to toggle the panel.
pub fn send_toggle(database: &str) -> bool {
    send(database, "toggle")
}

/// Starts listening. Returns false when another instance already owns the
/// socket, in which case this process should have asked it to toggle instead.
pub fn listen(database: &str, tx: UnboundedSender<IpcCommand>) -> bool {
    let path = socket_path(database);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            // A crashed instance can leave the file behind; if nobody answers,
            // take the socket over.
            if send_toggle(database) {
                return false;
            }
            let _ = std::fs::remove_file(&path);
            match UnixListener::bind(&path) {
                Ok(listener) => listener,
                Err(_) => return false,
            }
        }
        Err(_) => return false,
    };

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_err() {
                continue;
            }
            let Some(command) = parse(line.trim()) else {
                continue;
            };
            if tx.unbounded_send(command).is_err() {
                return;
            }
        }
    });

    true
}

/// One line of the control protocol.
fn parse(line: &str) -> Option<IpcCommand> {
    let mut parts = line.split_whitespace();
    match parts.next()? {
        "toggle" => Some(IpcCommand::Toggle),
        "pair" => Some(IpcCommand::Pair(parts.next()?.to_string())),
        "accept" => Some(IpcCommand::Accept {
            peer_id: parts.next()?.to_string(),
            pin: parts.next()?.to_string(),
        }),
        "connect" => Some(IpcCommand::Connect(parts.next()?.to_string())),
        _ => None,
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_lines_map_to_commands() {
        assert!(matches!(parse("toggle"), Some(IpcCommand::Toggle)));
        assert!(matches!(parse("pair 12D3Koo"), Some(IpcCommand::Pair(id)) if id == "12D3Koo"));
        match parse("accept 12D3Koo 123456") {
            Some(IpcCommand::Accept { peer_id, pin }) => {
                assert_eq!(peer_id, "12D3Koo");
                assert_eq!(pin, "123456");
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(matches!(
            parse("connect /ip4/1.2.3.4/tcp/1"),
            Some(IpcCommand::Connect(_))
        ));
        assert!(parse("nonsense").is_none());
    }
}
