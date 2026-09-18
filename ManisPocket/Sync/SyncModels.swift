import Foundation

struct PairedDeviceInfo: Codable, Identifiable, Equatable {
    var id: String {
        peerID
    }

    let peerID: String
    var nickname: String
    var icon: String
    var connectedAt: Date
    var isConnected: Bool
    var isAdmin: Bool

    /// Load paired devices from Rust core (SQLite), not UserDefaults.
    static func loadFromCore() -> [PairedDeviceInfo] {
        guard let jsonList = try? AppState.shared.history.core.getPairedPeers() else { return [] }
        return jsonList.compactMap { json in
            guard let data = json.data(using: .utf8),
                  let dict = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let peerId = dict["peerId"] as? String,
                  let name = dict["displayName"] as? String else { return nil }
            // isAdmin/isOnline are JSON booleans from the Rust core, not strings.
            let boolValue = { (key: String) -> Bool in
                if let v = dict[key] as? Bool {
                    return v
                }
                return (dict[key] as? String) == "true"
            }
            return PairedDeviceInfo(
                peerID: peerId,
                nickname: name,
                icon: "💻",
                connectedAt: Date(),
                isConnected: boolValue("isOnline"),
                isAdmin: boolValue("isAdmin")
            )
        }
    }
}

struct SyncItemContent: Codable {
    let type: String
    let value: String?
}

struct SyncItem: Codable {
    let id: String
    let application: String?
    let firstCopiedAt: String
    let lastCopiedAt: String
    let numberOfCopies: Int
    let pin: String?
    let title: String
    let contents: [SyncItemContent]
    let syncTimestamp: String
    let syncSource: String
}
