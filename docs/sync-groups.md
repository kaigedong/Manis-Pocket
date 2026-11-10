# Sync Group & Permission Design

## Concepts

A **group** is a set of devices that have paired with each other.  
An **admin** is the device that controls membership.

## Rules

1. **Only one admin per group.** The admin can transfer the role to another member.
2. **First to connect is admin.** The device that initiates pairing (clicks "Pair") becomes admin.
3. **A device can only belong to one group.** It must leave its current group before joining another.
4. **Only admin can remove members.** Regular members cannot kick anyone.
5. **Admin transfer.** Admin can promote any member to admin, which demotes the current admin.

## Data Model (Rust SQLite)

```sql
-- sync_pairs table (v3)
CREATE TABLE sync_pairs (
    peer_id     TEXT PRIMARY KEY,  -- libp2p peer ID
    display_name TEXT NOT NULL,    -- human-readable device name
    is_admin    INTEGER DEFAULT 0, -- 1 if this peer is admin
    is_online   INTEGER DEFAULT 0  -- 1 if currently connected
);
```

## APIs (HistoryManager)

| Method | Description |
|--------|-------------|
| `get_paired_peers() -> [JSON]` | Returns `peerId`, `displayName`, `isAdmin`, `isOnline` |
| `save_paired_peer(peer_id, name, is_admin)` | Add/update a paired peer |
| `remove_paired_peer(peer_id)` | Remove a peer (only works if local device is admin) |
| `promote_to_admin(peer_id)` | Transfer admin to this peer |

## Lifecycle

```
Device A                         Device B
────────                         ────────
Click "Pair" on B
  → save_paired_peer(B, admin=true)
  → send PairingRequest          → receives PairingRequest
                                 → shows PIN dialog
                                 → clicks "Confirm"
                                 → save_paired_peer(A, admin=false)
  ← receives Accept              → sends Accept
  → pairing complete
```

Device A is now admin. Device B is a regular member.
A can see B in Paired Devices with "Unpair" button.
B can see A in Paired Devices but NO "Unpair" button.
A can transfer admin to B via promote_to_admin(B).

## Joining a new group

If B wants to join C's group:
1. B must first leave A's group (A unpairs B, or B unpairs itself if it's admin)
2. Then C can pair with B
