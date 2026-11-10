import Foundation
import SwiftData

@MainActor
class Storage {
    static let shared = Storage()

    var container: ModelContainer
    var context: ModelContext {
        container.mainContext
    }

    var size: String {
        guard let size = try? url.resourceValues(forKeys: [.fileSizeKey]).allValues.first?.value as? Int64, size > 1 else {
            return ""
        }

        return ByteCountFormatter().string(fromByteCount: size)
    }

    private let url = URL.applicationSupportDirectory.appending(path: "ManisPocket/Storage.sqlite")

    init() {
        if !FileManager.default.fileExists(atPath: url.path) {
            try? FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            let legacy = URL.applicationSupportDirectory.appending(path: "Maccy/Storage.sqlite")
            for suffix in ["", "-wal", "-shm"] {
                let source = URL(fileURLWithPath: legacy.path + suffix)
                let target = URL(fileURLWithPath: url.path + suffix)
                if FileManager.default.fileExists(atPath: source.path) {
                    try? FileManager.default.copyItem(at: source, to: target)
                }
            }
        }
        var config = ModelConfiguration(url: url)

        #if DEBUG
            if CommandLine.arguments.contains("enable-testing") {
                config = ModelConfiguration(isStoredInMemoryOnly: true)
            }
        #endif

        do {
            container = try ModelContainer(for: HistoryItem.self, configurations: config)
        } catch {
            fatalError("Cannot load database: \(error.localizedDescription).")
        }
    }
}
