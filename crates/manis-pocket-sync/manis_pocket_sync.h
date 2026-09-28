#include <cstdarg>
#include <cstdint>
#include <cstdlib>
#include <ostream>
#include <new>

/// Keep the complete CBOR request comfortably below libp2p's default 1 MiB limit.
constexpr static const uintptr_t MAX_CLIPBOARD_TEXT_BYTES = (512 * 1024);

/// Fixed listen port for reliable reconnection.
constexpr static const uint16_t LISTEN_PORT = 31774;

template<typename T = void>
struct Arc;

struct SyncState;

using ManisPocketSync = Arc<Mutex<SyncState>>;

extern "C" {

ManisPocketSync *manis_pocket_sync_create(const char *device_name, const char *device_id);

void manis_pocket_sync_destroy(ManisPocketSync *sync);

int32_t manis_pocket_sync_start(ManisPocketSync *sync);

int32_t manis_pocket_sync_stop(ManisPocketSync *sync);

void manis_pocket_sync_on_event(ManisPocketSync *sync, void (*cb)(const char *event_json));

int32_t manis_pocket_sync_start_discovery(ManisPocketSync *sync);

int32_t manis_pocket_sync_stop_discovery(ManisPocketSync *sync);

int32_t manis_pocket_sync_add_peer_address(ManisPocketSync *sync,
                                           const char *_peer_id,
                                           const char *address);

int32_t manis_pocket_sync_request_pairing(ManisPocketSync *sync, const char *peer_id);

int32_t manis_pocket_sync_accept_pairing(ManisPocketSync *sync,
                                         const char *peer_id,
                                         const char *pin);

int32_t manis_pocket_sync_reject_pairing(ManisPocketSync *sync, const char *peer_id);

int32_t manis_pocket_sync_unpair(ManisPocketSync *sync, const char *peer_id);

int32_t manis_pocket_sync_broadcast_item(ManisPocketSync *sync, const char *item_json);

int32_t manis_pocket_sync_broadcast_deletion(ManisPocketSync *sync, const char *item_id);

int32_t manis_pocket_sync_broadcast_update(ManisPocketSync *sync, const char *item_json);

char *manis_pocket_sync_get_paired_peers(ManisPocketSync *sync);

void manis_pocket_sync_free_string(char *s);

bool manis_pocket_sync_is_running(ManisPocketSync *sync);

char *manis_pocket_sync_get_status(ManisPocketSync *sync);

}  // extern "C"
