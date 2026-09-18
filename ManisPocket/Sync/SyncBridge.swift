import AppKit
import Defaults
import Foundation

/// macOS implementation of the `ClipboardObserver` protocol from Rust.
/// Receives sync events and routes them to the UI layer.
///
/// Architecture:
///   HistoryManager (Rust) → SyncEngine → ClipboardObserver (this class)
///       → History.shared.add() / NotificationCenter → SwiftUI
class ManisPocketSyncObserver: ClipboardObserver {
    func onItemReceived(item: ClipboardItem) {
        Task { @MainActor in
            // MUST set this before add(): add() reads it to decide whether to
            // rebroadcast. Setting it afterwards (as it used to be) let every
            // received item be echoed back, causing runaway duplicate broadcasts.
            History.shared.syncBroadcastToPeers = false
            NSLog("[Sync] onItemReceived id=\(item.id) title=\(item.title.prefix(40)) contents=\(item.contents.map { "\($0.contentType):\($0.value?.count ?? 0)" })")
            _ = History.shared.add(item, shouldAppend: false)
        }
    }

    func onItemDeleted(itemId: String) {
        Task { @MainActor in
            History.shared.deleteBySyncID(itemId)
        }
    }

    func onItemUpdated(item: ClipboardItem) {
        Task { @MainActor in
            History.shared.updateBySyncID(item)
        }
    }

    func onPeerDiscovered(peerId: String, displayName: String, addresses: [String], isConnected: Bool) {
        DispatchQueue.main.async {
            if isConnected {
                Notifier.notify(body: "\(displayName) connected", sound: .knock)
                NotificationCenter.default.post(name: NSNotification.Name("showSyncSettings"), object: nil)
            }
            NotificationCenter.default.post(name: .syncPeerDiscovered, object: nil, userInfo: [
                "peerID": peerId,
                "displayName": displayName,
                "addresses": addresses,
                "isConnected": isConnected,
            ])
        }
    }

    func onPeerLost(peerId: String) {
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: .syncPeerLost, object: nil, userInfo: [
                "peerID": peerId,
            ])
        }
    }

    func onPairingRequest(peerId: String, displayName: String, pin: String) {
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: .syncPairingRequest, object: nil, userInfo: [
                "peerID": peerId,
                "displayName": displayName,
                "pin": pin,
            ])
        }
    }

    func onPairingComplete(peerId: String, success: Bool) {
        DispatchQueue.main.async {
            if success {
                SyncBridge.shared.recordPeerName(peerId, peerId) // name will be updated by peer_discovered
                AppState.shared.history.core.savePairedPeer(peerId: peerId, displayName: peerId, isAdmin: true)
            } else {
                AppState.shared.history.core.removePairedPeer(peerId: peerId)
            }
            NotificationCenter.default.post(name: .syncPairingComplete, object: nil, userInfo: [
                "peerID": peerId,
                "success": success,
            ])
        }
    }

    func onListening(address: String) {
        NSLog("[Sync] listening on \(address)")
    }

    func onError(code: Int32, message: String) {
        NSLog("[Sync] error (\(code)): \(message)")
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: .syncError, object: nil, userInfo: [
                "code": code,
                "message": message,
            ])
        }
    }

    func onFileChunk(requestId: String, fileName: String, fileSize: Int64, chunkIndex: Int32, totalChunks: Int32, data: Data) {
        FileDownloadManager.shared.receiveChunk(
            requestId: requestId,
            fileName: fileName,
            fileSize: fileSize,
            chunkIndex: chunkIndex,
            totalChunks: totalChunks,
            data: data
        )
    }

    func onFileDownloadComplete(requestId: String, filePath: String, success: Bool) {
        FileDownloadManager.shared.complete(requestId: requestId, filePath: filePath, success: success)
    }
}

@MainActor
class SyncBridge {
    static let shared = SyncBridge()

    private(set) var peerDisplayNames: [String: String] = [:]
    private var isStarted = false

    private init() {}

    var isEnabled: Bool {
        Defaults[.syncEnabled]
    }

    func start() {
        guard !isStarted else { return }
        guard isEnabled else { return }

        let deviceName = Defaults[.syncDeviceName]
        let deviceID = Defaults[.syncDeviceID]
        NSLog("[Sync] start: name=\(deviceName)")

        let observer = ManisPocketSyncObserver()
        do {
            try AppState.shared.history.core.startSync(
                deviceName: deviceName,
                deviceId: deviceID,
                observer: observer
            )
        } catch {
            NSLog("[Sync] start failed: \(error)")
            return
        }

        isStarted = true
        NSLog("[Sync] started (via HistoryManager)")
    }

    func stop() {
        guard isStarted else { return }
        do {
            try AppState.shared.history.core.stopSync()
        } catch {
            NSLog("[Sync] stop failed: \(error)")
        }
        isStarted = false
    }

    // ── Peer discovery events (cache names) ────────────────────────

    func recordPeerName(_ peerID: String, _ name: String) {
        peerDisplayNames[peerID] = name
    }

    // ── Thin wrappers that delegate to HistoryManager ──────────────

    func addPeerAddress(_ address: String) {
        AppState.shared.history.core.syncAddPeerAddress(address: address)
    }

    func requestPairing(peerID: String) {
        AppState.shared.history.core.syncRequestPairing(peerId: peerID)
    }

    func acceptPairing(peerID: String, pin: String) {
        AppState.shared.history.core.syncAcceptPairing(peerId: peerID, pin: pin)
    }

    func rejectPairing(peerID: String) {
        AppState.shared.history.core.syncRejectPairing(peerId: peerID)
    }

    func unpair(peerID: String) {
        AppState.shared.history.core.syncUnpair(peerId: peerID)
    }

    func broadcastNewItem(_ item: ClipboardItem) {
        AppState.shared.history.core.syncBroadcastItem(item: item)
    }

    func broadcastDeletion(_ syncID: UUID) {
        AppState.shared.history.core.syncBroadcastDeletion(itemId: syncID.uuidString)
    }

    func broadcastUpdate(_ item: ClipboardItem) {
        AppState.shared.history.core.syncBroadcastUpdate(item: item)
    }

    func requestFile(peerId: String, filePath: String) {
        AppState.shared.history.core.requestFile(peerId: peerId, filePath: filePath)
    }

    func refreshDiscovery() {
        AppState.shared.history.core.syncRefreshDiscovery()
    }
}

extension Notification.Name {
    static let syncPeerDiscovered = Notification.Name("syncPeerDiscovered")
    static let syncPeerLost = Notification.Name("syncPeerLost")
    static let syncPairingRequest = Notification.Name("syncPairingRequest")
    static let syncPairingComplete = Notification.Name("syncPairingComplete")
    static let syncItemReceived = Notification.Name("syncItemReceived")
    static let syncItemDeleted = Notification.Name("syncItemDeleted")
    static let syncItemUpdated = Notification.Name("syncItemUpdated")
    static let syncError = Notification.Name("syncError")
    static let fileDownloadComplete = Notification.Name("fileDownloadComplete")
}

/// Assembles file chunks received via P2P and saves to ~/Downloads.
class FileDownloadManager {
    static let shared = FileDownloadManager()
    private var downloads: [String: FileDownload] = [:]

    struct FileDownload {
        let requestId: String
        let fileName: String
        let totalSize: Int64
        var data = Data()
    }

    func receiveChunk(requestId: String, fileName: String, fileSize: Int64, chunkIndex _: Int32, totalChunks _: Int32, data: Data) {
        if downloads[requestId] == nil {
            downloads[requestId] = FileDownload(requestId: requestId, fileName: fileName, totalSize: fileSize)
        }
        downloads[requestId]?.data.append(data)
    }

    func complete(requestId: String, filePath _: String, success: Bool) {
        guard success, let dl = downloads.removeValue(forKey: requestId) else {
            downloads.removeValue(forKey: requestId)
            return
        }
        let url = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Downloads")
            .appendingPathComponent(dl.fileName)
        try? dl.data.write(to: url)
        NSLog("[FileTransfer] downloaded \(dl.fileName) (\(dl.data.count) bytes)")
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: .fileDownloadComplete, object: nil, userInfo: [
                "fileName": dl.fileName,
                "path": url.path,
                "size": dl.data.count,
            ])
            Notifier.notify(body: "Downloaded \(dl.fileName)", sound: .knock)
        }
    }
}
