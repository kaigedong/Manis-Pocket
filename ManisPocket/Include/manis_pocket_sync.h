#ifndef MANIS_POCKET_SYNC_H
#define MANIS_POCKET_SYNC_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ManisPocketSync ManisPocketSync;

/// Single unified callback — receives JSON-serialized SyncEvent.
typedef void (*ManisPocketEventCallback)(const char* event_json);

/// Legacy callbacks (kept for backward compat, now no-ops).
typedef void (*ManisPocketPeerDiscoveredCallback)(const char* peer_id, const char* display_name, const char* addresses);
typedef void (*ManisPocketPeerLostCallback)(const char* peer_id);
typedef void (*ManisPocketPairingRequestCallback)(const char* peer_id, const char* display_name, const char* pin);
typedef void (*ManisPocketPairingCompleteCallback)(const char* peer_id, bool success);
typedef void (*ManisPocketItemReceivedCallback)(const char* item_json);
typedef void (*ManisPocketItemDeletedCallback)(const char* item_id);
typedef void (*ManisPocketItemUpdatedCallback)(const char* item_json);
typedef void (*ManisPocketErrorCallback)(int32_t code, const char* message);

#define MANIS_POCKET_SYNC_OK              0
#define MANIS_POCKET_SYNC_ERR_INIT        1
#define MANIS_POCKET_SYNC_ERR_PAIRING     2
#define MANIS_POCKET_SYNC_ERR_NETWORK     3
#define MANIS_POCKET_SYNC_ERR_INVALID_ARG 4
#define MANIS_POCKET_SYNC_ERR_NOT_RUNNING 5

ManisPocketSync* manis_pocket_sync_create(const char* device_name, const char* device_id);
void manis_pocket_sync_destroy(ManisPocketSync* sync);

int32_t manis_pocket_sync_start(ManisPocketSync* sync);
int32_t manis_pocket_sync_stop(ManisPocketSync* sync);

/// Register the single unified event callback.
void manis_pocket_sync_on_event(ManisPocketSync* sync, ManisPocketEventCallback cb);

/// Legacy callbacks (no-ops, kept for ABI compat).
void manis_pocket_sync_on_peer_discovered(ManisPocketSync* sync, ManisPocketPeerDiscoveredCallback cb);
void manis_pocket_sync_on_peer_lost(ManisPocketSync* sync, ManisPocketPeerLostCallback cb);
void manis_pocket_sync_on_pairing_request(ManisPocketSync* sync, ManisPocketPairingRequestCallback cb);
void manis_pocket_sync_on_pairing_complete(ManisPocketSync* sync, ManisPocketPairingCompleteCallback cb);
void manis_pocket_sync_on_sync_item_received(ManisPocketSync* sync, ManisPocketItemReceivedCallback cb);
void manis_pocket_sync_on_sync_item_deleted(ManisPocketSync* sync, ManisPocketItemDeletedCallback cb);
void manis_pocket_sync_on_sync_item_updated(ManisPocketSync* sync, ManisPocketItemUpdatedCallback cb);
void manis_pocket_sync_on_error(ManisPocketSync* sync, ManisPocketErrorCallback cb);

int32_t manis_pocket_sync_start_discovery(ManisPocketSync* sync);
int32_t manis_pocket_sync_stop_discovery(ManisPocketSync* sync);

int32_t manis_pocket_sync_request_pairing(ManisPocketSync* sync, const char* peer_id);
int32_t manis_pocket_sync_accept_pairing(ManisPocketSync* sync, const char* peer_id, const char* pin);
int32_t manis_pocket_sync_reject_pairing(ManisPocketSync* sync, const char* peer_id);

int32_t manis_pocket_sync_broadcast_item(ManisPocketSync* sync, const char* item_json);
int32_t manis_pocket_sync_broadcast_deletion(ManisPocketSync* sync, const char* item_id);
int32_t manis_pocket_sync_broadcast_update(ManisPocketSync* sync, const char* item_json);

int32_t manis_pocket_sync_add_peer_address(ManisPocketSync* sync, const char* peer_id, const char* address);

char* manis_pocket_sync_get_paired_peers(ManisPocketSync* sync);
void manis_pocket_sync_free_string(char* s);
int32_t manis_pocket_sync_unpair(ManisPocketSync* sync, const char* peer_id);

bool manis_pocket_sync_is_running(ManisPocketSync* sync);
char* manis_pocket_sync_get_status(ManisPocketSync* sync);

#ifdef __cplusplus
}
#endif

#endif
