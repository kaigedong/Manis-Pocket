import AppKit
import Defaults
import Foundation

/// macOS implementation of the `ClipboardObserver` protocol from Rust.
/// Receives sync events and routes them to the UI layer.
///
/// Architecture:
///   HistoryManager (Rust) → SyncEngine → ClipboardObserver (this class)
///       → History.shared.add() / NotificationCenter → SwiftUI
final class ManisPocketSyncObserver: ClipboardObserver {
    private let notifyPeerConnected: @Sendable (String) -> Void

    init(notifyPeerConnected: @escaping @Sendable (String) -> Void = { displayName in
        Notifier.notify(body: "\(displayName) connected", sound: .knock)
    }) {
        self.notifyPeerConnected = notifyPeerConnected
    }

    func onItemReceived(item: ClipboardItem) {
        Task { @MainActor in
            guard AppState.shared.history.core.syncShouldApplyCurrentClipboard(eventId: item.id) else {
                AppState.shared.history.core.syncConfirmCurrentClipboard(eventId: item.id, success: false)
                return
            }
            if item.contents.isEmpty {
                Clipboard.shared.clearFromSync()
                AppState.shared.history.core.syncConfirmCurrentClipboard(eventId: item.id, success: true)
                return
            }
            var received = item
            // Linux and Android use MIME names; AppKit expects its string UTI.
            received.contents = item.contents.map { content in
                var normalized = content
                switch normalized.contentType {
                case "text/plain", "text/plain;charset=utf-8":
                    normalized.contentType = NSPasteboard.PasteboardType.string.rawValue
                case "image/png":
                    normalized.contentType = NSPasteboard.PasteboardType.png.rawValue
                default:
                    break
                }
                return normalized
            }
            // MUST set this before add(): add() reads it to decide whether to
            // rebroadcast. Setting it afterwards (as it used to be) let every
            // received item be echoed back, causing runaway duplicate broadcasts.
            History.shared.syncBroadcastToPeers = false
            History.shared.add(received, shouldAppend: false)
            History.shared.syncBroadcastToPeers = true
            Clipboard.shared.copy(received, fromSync: true)
            let applied = if let text = Clipboard.shared.getText(from: received) {
                NSPasteboard.general.string(forType: .string) == text
            } else if let image = Clipboard.shared.getImageData(from: received) {
                // The pasteboard was written with the exact bytes we received.
                NSPasteboard.general.data(forType: .png) == image
            } else {
                false
            }
            AppState.shared.history.core.syncConfirmCurrentClipboard(eventId: item.id, success: applied)
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
                self.notifyPeerConnected(displayName)
            }
            // Discovery and reconnection are background events, not requests to open settings.
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
        observeLocalClipboard(Self.currentClipboardContent())
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
        AppState.shared.history.core.syncBroadcastItem(item: item.withPortablePNG())
    }

    /// Report the system clipboard to the sync engine.
    ///
    /// `nil` means the clipboard is empty and is the only value that clears the
    /// peer. Content the protocol cannot share must not be reported at all.
    func observeLocalClipboard(_ content: ClipboardContent?) {
        guard isStarted else { return }
        AppState.shared.history.core.syncObserveLocalClipboard(content: content)
    }

    func observeLocalTextClipboard(_ text: String?) {
        guard let text, !text.isEmpty else {
            observeLocalClipboard(nil)
            return
        }
        observeLocalClipboard(ClipboardContent(
            contentType: NSPasteboard.PasteboardType.string.rawValue,
            value: Data(text.utf8)
        ))
    }

    /// PNG bytes for a macOS-native image payload (TIFF/JPEG/HEIC).
    nonisolated static func pngData(from data: Data) -> Data? {
        guard let image = NSImage(data: data),
              let tiff = image.tiffRepresentation,
              let representation = NSBitmapImageRep(data: tiff)
        else { return nil }
        return representation.representation(using: .png, properties: [:])
    }

    /// Current clipboard as a sync payload, or `nil` when it is empty.
    static func currentClipboardContent() -> ClipboardContent? {
        let pasteboard = NSPasteboard.general
        if let string = pasteboard.string(forType: .string), !string.isEmpty {
            return ClipboardContent(
                contentType: NSPasteboard.PasteboardType.string.rawValue,
                value: Data(string.utf8)
            )
        }
        if let png = pasteboard.data(forType: .png) {
            return ClipboardContent(
                contentType: NSPasteboard.PasteboardType.png.rawValue,
                value: png
            )
        }
        for type in [NSPasteboard.PasteboardType.tiff, .jpeg, .heic] {
            if let data = pasteboard.data(forType: type), let png = pngData(from: data) {
                return ClipboardContent(
                    contentType: NSPasteboard.PasteboardType.png.rawValue,
                    value: png
                )
            }
        }
        return nil
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

extension ClipboardItem {
    /// Add a `public.png` representation when the item only carries a
    /// macOS-native image format, so the sync engine can ship it to Linux.
    func withPortablePNG() -> ClipboardItem {
        let pngType = NSPasteboard.PasteboardType.png.rawValue
        if contents.contains(where: { $0.contentType == pngType }) {
            return self
        }
        let nativeTypes = [NSPasteboard.PasteboardType.tiff, .jpeg, .heic].map(\.rawValue)
        guard let source = contents.first(where: { nativeTypes.contains($0.contentType) })?.value,
              let png = SyncBridge.pngData(from: source)
        else { return self }
        var copy = self
        copy.contents = contents + [ClipboardContent(contentType: pngType, value: png)]
        return copy
    }
}
