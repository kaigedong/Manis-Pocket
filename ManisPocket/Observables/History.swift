import AppKit.NSRunningApplication
import Defaults
import Foundation
import Logging
import Observation
import Sauce
import Settings

@Observable
class History: ItemsContainer {
    static let shared = History()
    let logger = Logger(label: "com.kaigedong.manispocket")

    var items: [HistoryItemDecorator] = []
    var pasteStack: PasteStack?

    var pinnedItems: [HistoryItemDecorator] {
        items.filter(\.isPinned)
    }

    var unpinnedItems: [HistoryItemDecorator] {
        items.filter(\.isUnpinned)
    }

    var searchQuery: String = "" {
        didSet {
            throttler.throttle { [self] in
                let mode = mapSearchMode(Defaults[.searchMode])
                let results = core.search(query: searchQuery, items: filteredItems().map(\.item), mode: mode)
                updateItems(results)

                if searchQuery.isEmpty {
                    AppState.shared.navigator.select(item: unpinnedItems.first)
                } else {
                    AppState.shared.navigator.highlightFirst()
                }

                AppState.shared.popup.needsResize = true
            }
        }
    }

    var selectedApp: String? {
        didSet {
            throttler.throttle { [self] in
                let mode = mapSearchMode(Defaults[.searchMode])
                let results = core.search(query: searchQuery, items: filteredItems().map(\.item), mode: mode)
                updateItems(results)
                AppState.shared.popup.needsResize = true
            }
        }
    }

    func toggleAppSelection(_ bundleId: String) {
        selectedApp = selectedApp == bundleId ? nil : bundleId
    }

    var excludedDevices: Set<String> = [] {
        didSet {
            throttler.throttle { [self] in
                let mode = mapSearchMode(Defaults[.searchMode])
                let results = core.search(query: searchQuery, items: filteredItems().map(\.item), mode: mode)
                updateItems(results)
                AppState.shared.popup.needsResize = true
            }
        }
    }

    var sourceApps: [(bundleId: String, image: ApplicationImage)] {
        var seen = Set<String>()
        var result: [(bundleId: String, image: ApplicationImage)] = []
        var hasUnknown = false
        for item in all {
            if let bundleId = item.item.application, !bundleId.isEmpty, !seen.contains(bundleId) {
                seen.insert(bundleId)
                result.append((bundleId: bundleId, image: item.applicationImage))
            } else if item.item.application == nil || item.item.application?.isEmpty == true, !hasUnknown {
                hasUnknown = true
                result.append((bundleId: "", image: item.applicationImage))
            }
        }
        if result.isEmpty, !all.isEmpty {
            NSLog("[sourceApps] all=\(all.count) but result is empty — all items have application=nil?")
        }
        return result
    }

    var pressedShortcutItem: HistoryItemDecorator? {
        guard let event = NSApp.currentEvent else {
            return nil
        }

        let modifierFlags = event.modifierFlags
            .intersection(.deviceIndependentFlagsMask)
            .subtracting(.capsLock)

        guard HistoryItemAction(modifierFlags) != .unknown else {
            return nil
        }

        let key = Sauce.shared.key(for: Int(event.keyCode))
        return items.first { $0.shortcuts.contains(where: { $0.key == key }) }
    }

    private let throttler = Throttler(minimumDelay: 0.2)
    let core: HistoryManager

    @ObservationIgnored
    private var sessionLog: [Int: ClipboardItem] = [:]

    /// Set to false when processing a sync-received item to avoid echo.
    @ObservationIgnored
    var syncBroadcastToPeers = true

    @ObservationIgnored
    var all: [HistoryItemDecorator] = []

    init() {
        let dbPath = Self.databasePath
        core = try! HistoryManager(dbPath: dbPath)

        Task {
            for await _ in Defaults.updates(.pasteByDefault, initial: false) {
                updateShortcuts()
            }
        }

        Task {
            for await _ in Defaults.updates(.sortBy, initial: false) {
                try? await load()
            }
        }

        Task {
            for await _ in Defaults.updates(.pinTo, initial: false) {
                try? await load()
            }
        }

        Task {
            for await _ in Defaults.updates(.showSpecialSymbols, initial: false) {
                for item in items {
                    item.title = Clipboard.shared.generateTitle(for: item.item)
                }
            }
        }

        Task {
            for await _ in Defaults.updates(.imageMaxHeight, initial: false) {
                for item in items {
                    await item.cleanupImages()
                }
            }
        }
    }

    static var databasePath: String {
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let directory = appSupport.appendingPathComponent("ManisPocket")
        let destination = directory.appendingPathComponent("manispocket.db")
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        if !FileManager.default.fileExists(atPath: destination.path) {
            let legacy = appSupport.appendingPathComponent("Maccy/maccy.db")
            for suffix in ["", "-wal", "-shm"] {
                let source = URL(fileURLWithPath: legacy.path + suffix)
                let target = URL(fileURLWithPath: destination.path + suffix)
                if FileManager.default.fileExists(atPath: source.path) {
                    do {
                        try FileManager.default.copyItem(at: source, to: target)
                    } catch {
                        NSLog("[History] Failed to migrate database file: \(error)")
                    }
                }
            }
        }
        return destination.path
    }

    @MainActor
    func load() async throws {
        let rustItems = try core.load()
        let sortBy = mapSortBy(Defaults[.sortBy])
        let pinToTop = Defaults[.pinTo] != .bottom
        let sorted = core.sort(items: rustItems, sortBy: sortBy, pinToTop: pinToTop)
        all = sorted.map { HistoryItemDecorator($0) }
        items = filteredItems()

        limitHistorySize(to: Defaults[.size])
        updateShortcuts()
        Task {
            AppState.shared.popup.needsResize = true
        }
    }

    @MainActor
    private func limitHistorySize(to maxSize: Int) {
        let unpinned = all.filter(\.isUnpinned)
        if unpinned.count >= maxSize {
            unpinned[maxSize...].forEach(delete)
        }
    }

    @discardableResult
    @MainActor
    func add(_ item: ClipboardItem, shouldAppend: Bool = false) -> HistoryItemDecorator {
        if shouldAppend, !all.isEmpty {
            let unpinnedItems = all.filter { $0.item.pin == nil }
            let differentItems = unpinnedItems.filter { Clipboard.shared.getText(from: $0.item) != Clipboard.shared.getText(from: item) }

            if let mostRecentUnpinned = differentItems.max(by: { $0.item.lastCopiedAt < $1.item.lastCopiedAt }) {
                let topItem = mostRecentUnpinned.item

                if let existingText = Clipboard.shared.getText(from: topItem), let newText = Clipboard.shared.getText(from: item) {
                    let combinedText = existingText + "\n" + newText
                    let combinedData = combinedText.data(using: .utf8)

                    var updatedContents = topItem.contents
                    if let stringIdx = updatedContents.firstIndex(where: { $0.contentType == NSPasteboard.PasteboardType.string.rawValue }) {
                        updatedContents[stringIdx] = ClipboardContent(contentType: NSPasteboard.PasteboardType.string.rawValue, value: combinedData)
                    }

                    let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
                    var updatedItem = topItem
                    updatedItem.contents = updatedContents
                    updatedItem.lastCopiedAt = nowMs
                    updatedItem.numberOfCopies += 1
                    updatedItem.title = Clipboard.shared.generateTitle(for: updatedItem)
                    try? core.updateItemText(id: updatedItem.id, newText: combinedText)

                    mostRecentUnpinned.item = updatedItem
                    mostRecentUnpinned.title = updatedItem.title
                    items = all

                    Defaults[.ignoreOnlyNextEvent] = true
                    Defaults[.ignoreEvents] = true
                    Clipboard.shared.copy(combinedText)

                    return mostRecentUnpinned
                }
            }
        }

        // Add through Rust core (handles dedup, size limit, empty filtering)
        let maxSize = Defaults[.isUnlimitedHistory] ? 0 : Defaults[.size]
        guard let result = try? core.add(item: item, maxSize: Int32(maxSize), isUnlimited: Defaults[.isUnlimitedHistory]) else {
            return HistoryItemDecorator(item) // empty item rejected, return placeholder without storing
        }

        // Check if it was a dedup (same id returned) or new item
        let isNew = all.contains(where: { $0.item.id == result.id }) == false
        if isNew {
            Task {
                Notifier.notify(body: result.title, sound: .write)
            }
        }

        // Broadcast to synced peers (unless this item came from sync itself)
        if syncBroadcastToPeers {
            SyncBridge.shared.broadcastNewItem(result)
        }
        syncBroadcastToPeers = true

        sessionLog[Clipboard.shared.changeCount] = result

        var itemDecorator: HistoryItemDecorator
        if isNew {
            if let pin = result.pin {
                itemDecorator = HistoryItemDecorator(result, shortcuts: KeyShortcut.create(character: pin))
            } else {
                itemDecorator = HistoryItemDecorator(result)
            }
            all.insert(itemDecorator, at: 0)
        } else {
            // core.add merged this into an existing item (dedup). Refresh the
            // existing decorator in place instead of inserting a duplicate —
            // otherwise every re-receipt of the same item spawns a new row.
            itemDecorator = all.first(where: { $0.item.id == result.id }) ?? HistoryItemDecorator(result)
            itemDecorator.item = result
            itemDecorator.title = result.title
        }

        // Re-sort all items
        let sortBy = mapSortBy(Defaults[.sortBy])
        let pinToTop = Defaults[.pinTo] != .bottom
        let sorted = core.sort(items: all.map(\.item), sortBy: sortBy, pinToTop: pinToTop)
        reorderAll(matching: sorted)

        items = filteredItems()
        updateUnpinnedShortcuts()
        AppState.shared.popup.needsResize = true

        return itemDecorator
    }

    @MainActor
    func clear() {
        _ = try? core.clearUnpinned()
        all.removeAll(where: \.isUnpinned)
        sessionLog.removeValues { $0.pin == nil }
        items = filteredItems()

        Clipboard.shared.clear()
        AppState.shared.popup.close()
        Task {
            AppState.shared.popup.needsResize = true
        }
    }

    @MainActor
    func clearAll() {
        _ = try? core.clearAll()
        all.removeAll()
        sessionLog.removeAll()
        items = []

        Clipboard.shared.clear()
        AppState.shared.popup.close()
        Task {
            AppState.shared.popup.needsResize = true
        }
    }

    @MainActor
    func delete(_ item: HistoryItemDecorator?) {
        guard let item else { return }
        try? core.delete(id: item.item.id)

        cleanup(item)
        all.removeAll { $0 == item }
        items.removeAll { $0 == item }
        sessionLog.removeValues { $0.id == item.item.id }

        updateUnpinnedShortcuts()
        Task {
            AppState.shared.popup.needsResize = true
        }
    }

    /// Delete an item by its sync UUID (received from a peer).
    @MainActor
    func deleteBySyncID(_ syncID: String) {
        guard let item = all.first(where: { $0.item.syncSource == syncID || $0.item.id == syncID }) else { return }
        delete(item)
    }

    /// Update an item received from a synced peer.
    @MainActor
    func updateBySyncID(_ item: ClipboardItem) {
        syncBroadcastToPeers = false
        _ = add(item, shouldAppend: false)
    }

    @MainActor
    private func cleanup(_ item: HistoryItemDecorator) {
        item.cleanupImages()
    }

    private func currentModifierFlags() -> NSEvent.ModifierFlags {
        NSApp.currentEvent?.modifierFlags
            .intersection(.deviceIndependentFlagsMask)
            .subtracting([.capsLock, .numericPad, .function]) ?? []
    }

    @MainActor
    func select(_ item: HistoryItemDecorator?) {
        guard let item else { return }
        let modifierFlags = currentModifierFlags()

        if modifierFlags.isEmpty {
            AppState.shared.popup.close()
            Clipboard.shared.copy(item.item, removeFormatting: Defaults[.removeFormattingByDefault])
            if Defaults[.pasteByDefault] {
                Clipboard.shared.paste()
            }
        } else {
            switch HistoryItemAction(modifierFlags) {
            case .copy:
                AppState.shared.popup.close()
                Clipboard.shared.copy(item.item)
            case .paste:
                AppState.shared.popup.close()
                Clipboard.shared.copy(item.item)
                Clipboard.shared.paste()
            case .pasteWithoutFormatting:
                AppState.shared.popup.close()
                Clipboard.shared.copy(item.item, removeFormatting: true)
                Clipboard.shared.paste()
            case .unknown:
                return
            }
        }

        Task {
            searchQuery = ""
        }
    }

    @MainActor
    func startEditing(_ item: HistoryItemDecorator?) {
        guard let item, Clipboard.shared.getText(from: item.item) != nil else { return }
        item.editingText = Clipboard.shared.getText(from: item.item) ?? item.title
        item.isEditing = true
    }

    @MainActor
    func saveEditing(_ item: HistoryItemDecorator?) {
        guard let item, item.isEditing else { return }
        item.isEditing = false

        let newText = item.editingText
        guard !newText.isEmpty else {
            delete(item)
            return
        }

        _ = try? core.updateItemText(id: item.item.id, newText: newText)
        var updated = item.item
        updated.title = newText
        item.item = updated
        item.title = newText
    }

    @MainActor
    func cancelEditing(_ item: HistoryItemDecorator?) {
        guard let item, item.isEditing else { return }
        item.isEditing = false
        item.editingText = ""
        if item.item.title.isEmpty, (Clipboard.shared.getText(from: item.item) ?? "").isEmpty {
            delete(item)
        }
    }

    @discardableResult
    @MainActor
    func addNew() -> HistoryItemDecorator {
        let emptyContent = ClipboardContent(
            contentType: NSPasteboard.PasteboardType.string.rawValue,
            value: "".data(using: .utf8)
        )
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        var clipboardItem = ClipboardItem(
            id: UUID().uuidString,
            application: Bundle.main.bundleIdentifier,
            firstCopiedAt: nowMs,
            lastCopiedAt: nowMs,
            numberOfCopies: 1,
            pin: nil,
            title: "",
            contents: [emptyContent],
            syncTimestamp: nowMs,
            syncSource: nil,
            syncDeleted: false
        )
        clipboardItem.title = ""

        let decorator = add(clipboardItem)
        decorator.editingText = ""
        decorator.isEditing = true

        AppState.shared.navigator.select(item: decorator)
        return decorator
    }

    @MainActor
    func startPasteStack(selection: inout Selection<HistoryItemDecorator>) {
        guard AppState.shared.multiSelectionEnabled else { return }
        guard let item = selection.first else { return }
        PasteStack.initializeIfNeeded()

        let modifierFlags = currentModifierFlags()
        let stack = PasteStack(items: selection.items, modifierFlags: modifierFlags)
        pasteStack = stack

        if modifierFlags.isEmpty {
            AppState.shared.popup.close()
            Clipboard.shared.copy(item.item, removeFormatting: Defaults[.removeFormattingByDefault])
        } else {
            switch HistoryItemAction(modifierFlags) {
            case .copy:
                AppState.shared.popup.close()
                Clipboard.shared.copy(item.item)
            case .paste:
                AppState.shared.popup.close()
                Clipboard.shared.copy(item.item)
            case .pasteWithoutFormatting:
                AppState.shared.popup.close()
                Clipboard.shared.copy(item.item, removeFormatting: true)
                Clipboard.shared.paste()
            case .unknown:
                return
            }
        }

        Task {
            searchQuery = ""
        }
    }

    func handlePasteStack() {
        guard let stack = pasteStack else { return }
        guard let pasted = stack.items.first else {
            pasteStack = nil
            return
        }

        stack.items.removeFirst()

        guard let item = stack.items.first else {
            pasteStack = nil
            return
        }

        Task {
            if stack.modifierFlags.isEmpty {
                await Clipboard.shared.copy(item.item, removeFormatting: Defaults[.removeFormattingByDefault])
            } else {
                switch HistoryItemAction(stack.modifierFlags) {
                case .copy:
                    await Clipboard.shared.copy(item.item)
                case .paste:
                    await Clipboard.shared.copy(item.item)
                case .pasteWithoutFormatting:
                    await Clipboard.shared.copy(item.item, removeFormatting: true)
                case .unknown:
                    return
                }
            }
        }
    }

    func interruptPasteStack() {
        guard pasteStack != nil else { return }
        pasteStack = nil
    }

    @MainActor
    func togglePin(_ item: HistoryItemDecorator?) {
        guard let item else { return }

        let availablePins = Self.availablePins
        let result = try? core.togglePin(id: item.item.id, availablePins: availablePins)
        if let result {
            item.item = result
        }

        let sortBy = mapSortBy(Defaults[.sortBy])
        let pinToTop = Defaults[.pinTo] != .bottom
        let sorted = core.sort(items: all.map(\.item), sortBy: sortBy, pinToTop: pinToTop)
        reorderAll(matching: sorted)

        items = filteredItems()
        searchQuery = ""
        updateUnpinnedShortcuts()
        if item.isUnpinned {
            AppState.shared.navigator.scrollTarget = item.id
        }
    }

    // MARK: - Pin Helpers

    private static let supportedPins: Set<String> = {
        var keys = Set([
            "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l",
            "m", "n", "o", "p", "r", "s", "t", "u", "x", "y",
        ])

        if let deleteKey = KeyChord.deleteKey,
           let character = Sauce.shared.character(for: Int(deleteKey.QWERTYKeyCode), cocoaModifiers: [])
        {
            keys.remove(character)
        }
        if let pinKey = KeyChord.pinKey,
           let character = Sauce.shared.character(for: Int(pinKey.QWERTYKeyCode), cocoaModifiers: [])
        {
            keys.remove(character)
        }
        if let previewKey = KeyChord.previewKey,
           let character = Sauce.shared.character(for: Int(previewKey.QWERTYKeyCode), cocoaModifiers: [])
        {
            keys.remove(character)
        }

        return keys
    }()

    static var availablePins: [String] {
        let assignedPins = Set(History.shared.all.compactMap(\.item.pin))
        return Array(supportedPins.subtracting(assignedPins))
    }

    // MARK: - Private Helpers

    private func filteredItems() -> [HistoryItemDecorator] {
        if selectedApp == nil, excludedDevices.isEmpty {
            return all
        }

        return all.filter { item in
            let bundleId = item.item.application ?? ""
            if let selectedApp, bundleId != selectedApp {
                return false
            }
            if let syncSource = item.item.syncSource,
               excludedDevices.contains(syncSource)
            {
                return false
            }
            return true
        }
    }

    private func updateItems(_ results: [SearchResult]) {
        items = results.map { result in
            let decorator = all.first(where: { $0.item.id == result.item.id }) ?? HistoryItemDecorator(result.item)
            decorator.highlight(searchQuery, result.ranges)
            return decorator
        }

        updateUnpinnedShortcuts()
    }

    private func updateShortcuts() {
        for item in pinnedItems {
            if let pin = item.item.pin {
                item.shortcuts = KeyShortcut.create(character: pin)
            }
        }

        updateUnpinnedShortcuts()
    }

    private func updateUnpinnedShortcuts() {
        let visibleUnpinnedItems = unpinnedItems.filter(\.isVisible)
        for item in visibleUnpinnedItems {
            item.shortcuts = []
        }

        var index = 1
        for item in visibleUnpinnedItems.prefix(9) {
            item.shortcuts = KeyShortcut.create(character: String(index))
            index += 1
        }
    }

    private func reorderAll(matching sorted: [ClipboardItem]) {
        var itemMap: [String: HistoryItemDecorator] = [:]
        for decorator in all {
            itemMap[decorator.item.id] = decorator
        }
        all = sorted.compactMap { itemMap[$0.id] }
    }

    private func mapSearchMode(_ mode: Search.Mode) -> SearchMode {
        switch mode {
        case .exact: .exact
        case .fuzzy: .fuzzy
        case .regexp: .regexp
        case .mixed: .mixed
        }
    }

    private func mapSortBy(_ by: Sorter.By) -> SortBy {
        switch by {
        case .lastCopiedAt: .lastCopiedAt
        case .firstCopiedAt: .firstCopiedAt
        case .numberOfCopies: .numberOfCopies
        }
    }
}
