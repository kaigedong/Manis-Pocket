# Agent Integration (CLI & MCP)

Manis Pocket ships a built-in headless Agent CLI and a stdio Model Context Protocol (MCP) server inside the same `manis-pocket` binary.

- With **no arguments**, `manis-pocket` launches the GPUI desktop app and binds a same-user (`0o600`) local JSON Control API at `<data_dir>/agent.sock`.
- With a **subcommand**, `manis-pocket` talks to the running desktop app over `agent.sock` (live sync status + immediate UI refresh on mutations) and falls back cleanly to the local SQLite database and `settings.json` when the desktop app is offline.
- **Safety & context hygiene by default**:
  - `list` and `search` return bounded summaries (never multi-MB blobs).
  - `get` caps text length (`--max-chars`, default `32768`, with `truncated: true/false`) and returns image metadata (`width`, `height`, `size_bytes`, `sha256`) plus optional `--export-image <path>` instead of dumping raw base64 into agent context.
  - `add` enforces configured `ignore_regexp` rules so sensitive secrets matching ignore patterns are rejected.
  - `settings show` redacts raw `ignore_regexp` patterns (reporting `ignore_regexp_count`).
  - `peers` omits raw pairing PINs so device trust confirmation remains human-verified.

## CLI Quick Reference

Add `--json` to any subcommand for deterministic machine-readable JSON (`{ "ok", "source", "data" | "error" }`), and `--database <path>` to target a specific profile/test database:

```bash
# Runtime & storage overview (version, DB size, counts, live P2P sync state)
manis-pocket status --json

# Recent clipboard history summaries
manis-pocket list --limit 20 --json
manis-pocket list --kind image --limit 10 --json
manis-pocket list --pinned --json

# Search history (exact | fuzzy | regexp | mixed)
manis-pocket search "docker compose" --mode fuzzy --limit 10 --json

# Inspect full item text (bounded) or export a screenshot PNG to disk
manis-pocket get <id> --max-chars 8000 --json
manis-pocket get <id> --export-image /tmp/clip.png --json

# Restore an item back to the system clipboard
manis-pocket copy <id> --json

# Add a text snippet (pass '-' to read from stdin; respects ignore rules)
manis-pocket add "kubectl get pods -A" --pin --copy --json
echo "hello from pipe" | manis-pocket add - --json

# Idempotent pin / unpin and delete
manis-pocket pin <id> --json
manis-pocket unpin <id> --json
manis-pocket delete <id> --json

# P2P sync devices (paired, discovered, pending)
manis-pocket peers --json

# Inspect and update validated settings
manis-pocket settings show --json
manis-pocket settings set searchMode fuzzy --json
```

Installed binary paths:
- **macOS**: `/Applications/Manis Pocket.app/Contents/MacOS/manis-pocket`
- **Linux**: `/usr/bin/manis-pocket` (or `~/.local/bin/manis-pocket`)

## MCP Server (`manis-pocket mcp`)

Register `manis-pocket mcp` in any MCP-compatible AI client (Claude Code, Claude Desktop, Codex, Cursor, DSH, etc.):

```json
{
  "mcpServers": {
    "manis-pocket": {
      "command": "/Applications/Manis Pocket.app/Contents/MacOS/manis-pocket",
      "args": ["mcp"]
    }
  }
}
```

### Exposed Tools

| Tool | Mode | Purpose |
|---|---|---|
| `pocket_status` | Read-only | Version, platform, DB stats, item/pin counts, live sync state |
| `pocket_list_history` | Read-only | Bounded clipboard summaries with `limit`, `kind`, `app`, `pinned_only` filters |
| `pocket_search_history` | Read-only | Search clipboard history (`exact`, `fuzzy`, `regexp`, `mixed`) |
| `pocket_get_item` | Read-only | Full bounded text or image dimensions/SHA-256 + optional PNG export |
| `pocket_copy_item` | Write | Restore an existing item (text, image, file list) to the system clipboard |
| `pocket_add_text` | Write | Add text to history (honors ignore rules & dedup), optionally pin/copy |
| `pocket_set_pin` | Write (idempotent) | Pin or unpin an item by id |
| `pocket_delete_item` | Destructive | Permanently delete one item by id |
| `pocket_list_peers` | Read-only | Paired devices, live discovered LAN peers, pending pairings (no raw PINs) |
| `pocket_get_settings` | Read-only | Structured settings summary (redacts raw `ignore_regexp` patterns) |
| `pocket_update_setting` | Write (idempotent) | Validate and persist a supported setting key/value |
