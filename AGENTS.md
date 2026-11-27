# Manis Pocket Agent Guide (`AGENTS.md`)

Architectural rules, invariants, and verification commands for AI coding agents working in this repository.

## 1. Repository Map

| Path | Responsibility |
|---|---|
| `crates/manis-pocket-core` | Pure domain & persistence: SQLite storage, migrations, search (`exact`/`fuzzy`/`regexp`/`mixed`), sorting, deduplication, UniFFI bindings (`android/`). No UI. |
| `crates/manis-pocket-sync` | P2P sync network: libp2p (mDNS discovery, gossipsub, request-response), pairing protocol, C FFI. |
| `crates/manis-pocket-app` | GPUI desktop app (macOS + Linux): history panel, floating side image preview (`panel/preview_window.rs`), settings, tray, clipboard watcher, in-app self-updater (`app_update.rs`, `updater.rs`), local JSON Control API (`control_api.rs`), Agent CLI (`cli.rs`), and built-in stdio MCP server (`mcp.rs`). |
| `vendor/gpui-pre-macos` | Local patch fixing macOS IME candidate window caret rectangle. |
| `scripts/` & `packaging/` | macOS `.app` bundling, release versioning, Sparkle appcast scripts, Arch Linux & AUR packaging. |

Disclosed references (read when working in that branch):
- Threading model, sync dataflow, and platform differences: `docs/architecture.md`
- Build & packaging commands (macOS / Linux / Android): `docs/development.md`
- Headless Agent CLI & stdio MCP server contract: `docs/agent-integration.md`
- Sync group admin/pairing rules: `docs/sync-groups.md`

## 2. Hard Invariants

1. **Never read `ListState` inside its own `update` or `defer`**:
   - GPUI's entity borrow model aborts on re-entrant reads (`cannot read ListState while it is already being updated`).
   - Render from cached fields on `HistoryPanel` (`visible_count`, `selected_id`, `all_items`); push list-delegate callbacks to the panel via `cx.spawn` or `window.on_next_frame`.
2. **Isolate tests from the user's live database**:
   - Never touch `~/Library/Application Support/ManisPocket/manispocket.db` or `$XDG_DATA_HOME/manis-pocket/manispocket.db` during development or testing. Always use `:memory:` or a temporary `--database <path>` / `MANIS_POCKET_DB`.
3. **Bake release versions into binaries**:
   - `crates/manis-pocket-app/build.rs` falls back to `0.1.0.r<commits>.g<sha>` when `MANIS_POCKET_VERSION` is unset. Keep `MANIS_POCKET_VERSION` wired in `scripts/bundle-macos.sh`, `.github/workflows/build.yml`, and `packaging/archlinux/PKGBUILD` so `app_update` compares real release versions.
4. **Sync protocol safety & backwards compatibility**:
   - Unsupported or oversized (>20 MiB) local clipboard payloads must be skipped, never broadcast as empty/clear to peers. Preserve wire compatibility for older text-only peers.
5. **Agent surface security & bounded output**:
   - Bind `<data_dir>/control.sock` and `<data_dir>/agent.sock` with mode `0o600` (same-user only).
   - Keep `list`/`search` payloads bounded (`title_preview` <= 200 chars); return full text only via `get` (with `max_chars` cap and `truncated` flag); never emit raw image base64 into agent responses (return dimensions + SHA-256 + optional `--export-image <path>`).
   - Redact raw `ignore_regexp` patterns in settings responses, enforce ignore rules on `add`, and omit raw pairing PINs from agent peer listings.
6. **Non-blocking floating preview window**:
   - Image hover preview lives in `panel/preview_window.rs` as a non-activating `WindowKind::PopUp` (`focus: false`) placed beside the panel so vertical mouse movement across rows is unobstructed and closing the panel always dismisses it.

## 3. Agent Surface (`manis-pocket` CLI & MCP)

With no arguments, `manis-pocket` launches the GPUI desktop app and opens `<data_dir>/agent.sock` (`0o600`). With a subcommand, it runs headless (live socket with offline SQLite fallback) and accepts `--json` and `--database <path>`:

- `manis-pocket status --json` — version, DB stats, item/pin counts, live sync status
- `manis-pocket list [--limit N] [--kind text|image|file] [--app ...] [--pinned] --json`
- `manis-pocket search <query> [--mode exact|fuzzy|regexp|mixed] [--kind ...] [--app ...] [--limit N] --json`
- `manis-pocket get <id> [--max-chars N] [--export-image <path>] --json`
- `manis-pocket copy <id> --json` — restore entry to system clipboard
- `manis-pocket add <text|-> [--app ...] [--pin] [--copy] --json` — add text (honors ignore rules)
- `manis-pocket pin <id> --json` / `manis-pocket unpin <id> --json` — idempotent pin mutations
- `manis-pocket delete <id> --json`
- `manis-pocket peers --json` — paired, discovered, and pending sync devices (no raw PINs)
- `manis-pocket settings show --json` / `manis-pocket settings set <key> <value> --json`
- `manis-pocket mcp [--database <path>]` — stdio JSON-RPC 2.0 MCP server (11 tools)

## 4. Verification Commands

Run before committing any Rust or release-script change:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p manis-pocket-app
cargo test -p manis-pocket-core
cargo test -p manis-pocket-sync --lib
python3 -m unittest discover -s scripts/tests -v
```
