package com.kaigedong.manispocket

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import java.io.File

class HistoryViewModel : ViewModel() {
    private var core: HistoryManager? = null
    private var syncObserver: ClipboardObserver? = null
    private var appContext: Context? = null

    private val _items = MutableStateFlow<List<ClipboardItem>>(emptyList())
    val items: StateFlow<List<ClipboardItem>> = _items

    private val _searchResults = MutableStateFlow<List<SearchResult>>(emptyList())
    val searchResults: StateFlow<List<SearchResult>> = _searchResults

    // Sync state — discovered peers are ephemeral, paired peers are in Rust
    private val _peers = MutableStateFlow<List<DiscoveredPeer>>(emptyList())
    val peers: StateFlow<List<DiscoveredPeer>> = _peers

    private val _pairedPeers = MutableStateFlow<List<DiscoveredPeer>>(emptyList())
    val pairedPeers: StateFlow<List<DiscoveredPeer>> = _pairedPeers

    private val _pairingRequest = MutableStateFlow<PairingRequest?>(null)
    val pairingRequest: StateFlow<PairingRequest?> = _pairingRequest

    private val _syncError = MutableStateFlow<String?>(null)
    val syncError: StateFlow<String?> = _syncError

    private val _syncEnabled = MutableStateFlow(false)
    val syncEnabled: StateFlow<Boolean> = _syncEnabled

    fun initialize(context: Context) {
        appContext = context.applicationContext
        val dbPath = File(context.filesDir, "manispocket.db").absolutePath
        LogManager.i("History", "Opening database at $dbPath")
        try {
            core = HistoryManager(dbPath = dbPath)
            LogManager.i("History", "HistoryManager created successfully")
            loadItems()
        } catch (e: Exception) {
            LogManager.e("History", "Failed to create HistoryManager", e)
        }

        // Auto-start sync if it was enabled before, so users don't have to toggle
        // the switch every time the app is reopened.
        val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        if (prefs.getBoolean(KEY_SYNC_ENABLED, false)) {
            val name = prefs.getString(KEY_DEVICE_NAME, "Android Device") ?: "Android Device"
            LogManager.i("Sync", "Auto-starting sync (was enabled) as \"$name\"")
            startSync(name)
        }
    }

    // ── Sync ─────────────────────────────────────────────────────

    fun startSync(
        deviceName: String,
        deviceId: String? = null,
    ) {
        val core = this.core ?: return
        val id = deviceId ?: getOrCreateDeviceId(appContext!!)
        loadPairedPeers()
        syncObserver =
            ManisPocketClipboardObserver(
                onItemReceivedCb = { item ->
                    LogManager.i("Sync", "Received item: ${item.title.take(80)}")
                    viewModelScope.launch {
                        try {
                            core.add(item, maxSize = 500, isUnlimited = false)
                            loadItems()
                        } catch (e: Exception) {
                            LogManager.e("Sync", "Failed to add synced item", e)
                        }
                    }
                },
                onItemDeletedCb = { itemId ->
                    LogManager.d("Sync", "Remote delete: $itemId")
                    viewModelScope.launch {
                        try {
                            core.delete(itemId)
                            loadItems()
                        } catch (
                            e: Exception,
                        ) {
                            LogManager.e("Sync", "Failed to delete synced item", e)
                        }
                    }
                },
                onItemUpdatedCb = { item ->
                    LogManager.d("Sync", "Remote update: ${item.title.take(80)}")
                    viewModelScope.launch {
                        try {
                            core.add(item, maxSize = 500, isUnlimited = false)
                            loadItems()
                        } catch (e: Exception) {
                            LogManager.e("Sync", "Failed to update synced item", e)
                        }
                    }
                },
                onPeerDiscoveredCb = { peerId, displayName, addresses, isConnected ->
                    LogManager.i("Sync", "Peer: $displayName (connected=$isConnected)")
                    val list = _peers.value.toMutableList()
                    // Deduplicate by display name — replace old entry with same name
                    list.removeAll { it.displayName == displayName }
                    list.add(DiscoveredPeer(peerId, displayName, addresses, isConnected, false))
                    _peers.value = list
                    // Refresh paired-devices list so online/offline status updates live
                    // (isOnline is computed from the engine's connected set at read time).
                    loadPairedPeers()
                },
                onPeerLostCb = { peerId ->
                    _peers.value = _peers.value.filter { it.peerId != peerId }
                    loadPairedPeers()
                },
                onPairingRequestCb = { peerId, displayName, pin ->
                    LogManager.i("Sync", "Pairing request from $displayName (pin=$pin)")
                    _pairingRequest.value = PairingRequest(peerId, displayName, pin)
                },
                onPairingCompleteCb = { peerId, success ->
                    LogManager.i("Sync", "Pairing complete: peer=$peerId success=$success")
                    if (success) {
                        val name = _peers.value.find { it.peerId == peerId }?.displayName ?: peerId
                        core?.savePairedPeer(peerId, name, false) // responder is not admin
                    } else {
                        core?.removePairedPeer(peerId)
                    }
                    loadPairedPeers()
                    _pairingRequest.value = null
                },
                onListeningCb = { address ->
                    LogManager.i("Sync", "Listening on $address")
                },
                onErrorCb = { code, message ->
                    LogManager.e("Sync", "Error $code: $message")
                    _syncError.value = "Sync error: $message"
                },
                onFileChunkCb = { requestId, fileName, fileSize, chunkIndex, totalChunks, data ->
                    LogManager.d("FileTransfer", "Chunk $chunkIndex/$totalChunks of $fileName")
                    FileDownloadManager.receiveChunk(requestId, fileName, fileSize, chunkIndex, totalChunks, data)
                },
                onFileDownloadCompleteCb = { requestId, filePath, success ->
                    LogManager.i("FileTransfer", "Download complete: $filePath success=$success")
                    FileDownloadManager.complete(requestId, filePath, success)
                },
            )

        try {
            core.startSync(deviceName, id, syncObserver!!)
            _syncEnabled.value = true
            // Persist so sync auto-starts on next launch.
            appContext
                ?.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
                ?.edit()
                ?.putBoolean(KEY_SYNC_ENABLED, true)
                ?.putString(KEY_DEVICE_NAME, deviceName)
                ?.apply()
            // mDNS uses multicast; on Android we must hold a MulticastLock or the
            // WiFi driver drops the packets and we never discover peers.
            acquireMulticastLock()
            LogManager.i("Sync", "Sync started (via HistoryManager)")
        } catch (e: Exception) {
            LogManager.e("Sync", "Failed to start sync", e)
        }
    }

    fun stopSync() {
        try {
            core?.stopSync()
            _syncEnabled.value = false
            appContext
                ?.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
                ?.edit()
                ?.putBoolean(KEY_SYNC_ENABLED, false)
                ?.apply()
            releaseMulticastLock()
            LogManager.i("Sync", "Sync stopped")
        } catch (e: Exception) {
            LogManager.e("Sync", "Failed to stop sync", e)
        }
        syncObserver = null
    }

    // ── Multicast lock (required for mDNS on Android) ────────────

    private var multicastLock: android.net.wifi.WifiManager.MulticastLock? = null

    private fun acquireMulticastLock() {
        val ctx = appContext ?: return
        val wifi = ctx.getSystemService(Context.WIFI_SERVICE) as? android.net.wifi.WifiManager
        multicastLock =
            wifi?.createMulticastLock("manispocket-mdns")?.apply {
                setReferenceCounted(false)
                try {
                    acquire()
                    LogManager.i("Sync", "MulticastLock acquired")
                } catch (e: Exception) {
                    LogManager.e("Sync", "Failed to acquire MulticastLock", e)
                }
            }
    }

    private fun releaseMulticastLock() {
        multicastLock?.let { lock ->
            try {
                if (lock.isHeld) lock.release()
                LogManager.i("Sync", "MulticastLock released")
            } catch (e: Exception) {
                LogManager.e("Sync", "Failed to release MulticastLock", e)
            }
        }
        multicastLock = null
    }

    fun refreshDiscovery() {
        core?.syncRefreshDiscovery()
        LogManager.d("Sync", "Discovery refreshed")
    }

    fun loadPairedPeers() {
        val jsonList = core?.getPairedPeers() ?: emptyList()
        _pairedPeers.value =
            jsonList.mapNotNull { json ->
                try {
                    val obj = org.json.JSONObject(json)
                    val peerId = obj.getString("peerId")
                    val name = obj.getString("displayName")
                    val isAdmin = obj.optBoolean("isAdmin", false)
                    val isOnline = obj.optBoolean("isOnline", false)
                    DiscoveredPeer(peerId, name, emptyList(), isOnline, isAdmin)
                } catch (_: Exception) {
                    null
                }
            }
    }

    fun requestPairing(peerId: String) {
        core?.syncRequestPairing(peerId)
        LogManager.i("Sync", "Pairing requested with $peerId")
    }

    fun acceptPairing(
        peerId: String,
        pin: String,
    ) {
        core?.syncAcceptPairing(peerId, pin)
        LogManager.i("Sync", "Pairing accepted: $peerId")
    }

    fun rejectPairing(peerId: String) {
        core?.syncRejectPairing(peerId)
        _pairingRequest.value = null
    }

    fun unpair(peerId: String) {
        core?.syncUnpair(peerId)
        core?.removePairedPeer(peerId)
        loadPairedPeers()
        LogManager.i("Sync", "Unpaired: $peerId")
        _peers.value = _peers.value.filter { it.peerId != peerId }
    }

    fun requestFile(
        peerId: String,
        filePath: String,
    ) {
        core?.requestFile(peerId, filePath)
        LogManager.i("FileTransfer", "Requesting file: $filePath from $peerId")
    }

    fun addPeerAddress(address: String) {
        core?.syncAddPeerAddress(address)
        LogManager.i("Sync", "Dialing: $address")
    }

    // ── History ──────────────────────────────────────────────────

    fun loadItems() {
        viewModelScope.launch {
            core?.let { manager ->
                try {
                    _items.value = manager.load()
                } catch (e: Exception) {
                    LogManager.e("History", "Failed to load items", e)
                    _items.value = emptyList()
                }
            }
        }
    }

    fun addItem(item: ClipboardItem) {
        viewModelScope.launch {
            core?.let { manager ->
                try {
                    val result = manager.add(item, maxSize = 500, isUnlimited = false)
                    // Only broadcast genuinely-new items. core.add returns the same id
                    // for a new item but a different (existing) id when it deduped —
                    // rebroadcasting the deduped item would loop the same item to peers.
                    if (result.id == item.id) {
                        manager.syncBroadcastItem(result)
                        LogManager.i("Sync", "Broadcast new item id=${item.id.take(8)} title=${item.title.take(40)}")
                    } else {
                        LogManager.i("Sync", "Skipped broadcast (dedup of ${result.id.take(8)}): ${item.title.take(40)}")
                    }
                    LogManager.d("History", "Added item: ${item.id.take(8)}...")
                } catch (e: Exception) {
                    LogManager.e("History", "Failed to add item", e)
                }
                loadItems()
            }
        }
    }

    fun deleteItem(id: String) {
        viewModelScope.launch {
            core?.let { manager ->
                try {
                    manager.delete(id)
                    manager.syncBroadcastDeletion(id)
                    LogManager.d("History", "Deleted item: $id")
                } catch (e: Exception) {
                    LogManager.e("History", "Failed to delete item", e)
                }
                loadItems()
            }
        }
    }

    fun togglePin(id: String) {
        viewModelScope.launch {
            core?.let { manager ->
                try {
                    manager.togglePin(id, listOf("b", "c", "d", "e", "f", "g", "h", "i", "j", "k"))
                } catch (e: Exception) {
                    LogManager.e("History", "Failed to toggle pin", e)
                }
                loadItems()
            }
        }
    }

    fun search(
        query: String,
        mode: SearchMode = SearchMode.MIXED,
    ) {
        viewModelScope.launch {
            core?.let { manager ->
                try {
                    _searchResults.value = manager.search(query, _items.value, mode)
                } catch (e: Exception) {
                    LogManager.e("History", "Search failed", e)
                }
            }
        }
    }

    fun clearError() {
        _syncError.value = null
    }

    fun dismissPairingRequest() {
        _pairingRequest.value = null
    }

    override fun onCleared() {
        super.onCleared()
        stopSync()
        LogManager.i("History", "ViewModel cleared")
        core = null
    }

    companion object {
        private const val PREFS_NAME = "manis_pocket_sync"
        private const val KEY_DEVICE_ID = "device_id"
        private const val KEY_SYNC_ENABLED = "sync_enabled"
        private const val KEY_DEVICE_NAME = "device_name"

        fun getOrCreateDeviceId(context: Context): String {
            val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            var id = prefs.getString(KEY_DEVICE_ID, null)
            if (id == null) {
                id =
                    java.util.UUID
                        .randomUUID()
                        .toString()
                prefs.edit().putString(KEY_DEVICE_ID, id).apply()
                LogManager.i("Sync", "Created persistent device ID: $id")
            }
            return id
        }
    }
}

// ── Sync data classes ────────────────────────────────────────────

data class DiscoveredPeer(
    val peerId: String,
    val displayName: String,
    val addresses: List<String>,
    val isConnected: Boolean,
    val isAdmin: Boolean = false,
)

data class PairingRequest(
    val peerId: String,
    val displayName: String,
    val pin: String,
)

// ── ClipboardObserver UniFFI implementation ──────────────────────

class ManisPocketClipboardObserver(
    private val onItemReceivedCb: (ClipboardItem) -> Unit,
    private val onItemDeletedCb: (String) -> Unit,
    private val onItemUpdatedCb: (ClipboardItem) -> Unit,
    private val onPeerDiscoveredCb: (peerId: String, displayName: String, addresses: List<String>, isConnected: Boolean) -> Unit,
    private val onPeerLostCb: (String) -> Unit,
    private val onPairingRequestCb: (peerId: String, displayName: String, pin: String) -> Unit,
    private val onPairingCompleteCb: (peerId: String, success: Boolean) -> Unit,
    private val onListeningCb: (String) -> Unit,
    private val onErrorCb: (code: Int, message: String) -> Unit,
    private val onFileChunkCb: (
        requestId: String,
        fileName: String,
        fileSize: Long,
        chunkIndex: Int,
        totalChunks: Int,
        data: ByteArray,
    ) -> Unit,
    private val onFileDownloadCompleteCb: (requestId: String, filePath: String, success: Boolean) -> Unit,
) : ClipboardObserver {
    override fun onItemReceived(item: ClipboardItem) = onItemReceivedCb(item)

    override fun onItemDeleted(itemId: String) = onItemDeletedCb(itemId)

    override fun onItemUpdated(item: ClipboardItem) = onItemUpdatedCb(item)

    override fun onPeerDiscovered(
        peerId: String,
        displayName: String,
        addresses: List<String>,
        isConnected: Boolean,
    ) = onPeerDiscoveredCb(peerId, displayName, addresses, isConnected)

    override fun onPeerLost(peerId: String) = onPeerLostCb(peerId)

    override fun onPairingRequest(
        peerId: String,
        displayName: String,
        pin: String,
    ) = onPairingRequestCb(peerId, displayName, pin)

    override fun onPairingComplete(
        peerId: String,
        success: Boolean,
    ) = onPairingCompleteCb(peerId, success)

    override fun onListening(address: String) = onListeningCb(address)

    override fun onError(
        code: Int,
        message: String,
    ) = onErrorCb(code, message)

    override fun onFileChunk(
        requestId: String,
        fileName: String,
        fileSize: Long,
        chunkIndex: Int,
        totalChunks: Int,
        data: ByteArray,
    ) = onFileChunkCb(requestId, fileName, fileSize, chunkIndex, totalChunks, data)

    override fun onFileDownloadComplete(
        requestId: String,
        filePath: String,
        success: Boolean,
    ) = onFileDownloadCompleteCb(requestId, filePath, success)
}
