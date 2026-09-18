import AppKit.NSWorkspace
import Defaults
import Foundation
import Observation
import Sauce

@Observable
class HistoryItemDecorator: Identifiable, Hashable, HasVisibility {
    static func == (lhs: HistoryItemDecorator, rhs: HistoryItemDecorator) -> Bool {
        lhs.id == rhs.id
    }

    static var previewImageSize: NSSize {
        NSScreen.forPopup?.visibleFrame.size ?? NSSize(width: 2048, height: 1536)
    }

    static var thumbnailImageSize: NSSize {
        NSSize(width: 340, height: Defaults[.imageMaxHeight])
    }

    let id = UUID()

    var title: String = ""
    var attributedTitle: AttributedString?

    var isVisible: Bool = true
    var selectionIndex: Int = -1
    var isSelected: Bool {
        selectionIndex != -1
    }

    var shortcuts: [KeyShortcut] = []
    var isEditing: Bool = false
    var editingText: String = ""

    var application: String? {
        let universalClipboardTypes = ["com.apple.UIKit.pboardName"]
        let hasUniversal = item.contents.contains(where: { universalClipboardTypes.contains($0.contentType) })
        if hasUniversal {
            return "iCloud"
        }

        guard let bundle = item.application,
              let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle)
        else {
            return nil
        }

        return url.deletingPathExtension().lastPathComponent
    }

    var hasImage: Bool {
        imageData != nil
    }

    var previewImageGenerationTask: Task<Void, Error>?
    var thumbnailImageGenerationTask: Task<Void, Error>?
    var previewImage: NSImage?
    var thumbnailImage: NSImage?
    var applicationImage: ApplicationImage

    var text: String {
        Clipboard.shared.getPreviewableText(from: item).shortened(to: 10000)
    }

    var isPinned: Bool {
        item.pin != nil
    }

    var isUnpinned: Bool {
        item.pin == nil
    }

    /// True if this is a file reference (copied from Finder).
    var isFile: Bool {
        item.contents.contains { $0.contentType == NSPasteboard.PasteboardType.fileURL.rawValue }
    }

    /// File name extracted from the file URL, if this is a file item.
    var fileName: String? {
        guard isFile else { return nil }
        return item.contents
            .first { $0.contentType == NSPasteboard.PasteboardType.fileURL.rawValue }
            .flatMap(\.value)
            .flatMap { URL(dataRepresentation: Data($0), relativeTo: nil, isAbsolute: true) }
            .map { $0.lastPathComponent.removingPercentEncoding ?? $0.lastPathComponent }
    }

    /// True if this item came from a remote device (sync).
    var isRemote: Bool {
        item.syncSource != nil && !item.syncSource!.isEmpty
    }

    func hash(into hasher: inout Hasher) {
        hasher.combine(id)
        hasher.combine(title)
        hasher.combine(attributedTitle)
    }

    var item: ClipboardItem

    /// Computed AppKit properties derived from ClipboardItem contents
    var imageData: Data? {
        let imageTypes = [NSPasteboard.PasteboardType.tiff, .png, .jpeg, .heic].map(\.rawValue)
        guard let content = item.contents.first(where: { imageTypes.contains($0.contentType) }),
              let value = content.value else { return nil }
        return Data(value)
    }

    var image: NSImage? {
        guard let data = imageData else { return nil }
        return NSImage(data: data)
    }

    init(_ item: ClipboardItem, shortcuts: [KeyShortcut] = []) {
        self.item = item
        self.shortcuts = shortcuts
        title = item.title
        applicationImage = ApplicationImageCache.shared.getImage(application: item.application)
    }

    @MainActor
    func ensureThumbnailImage() {
        guard image != nil else { return }
        guard thumbnailImage == nil else { return }
        guard thumbnailImageGenerationTask == nil else { return }
        thumbnailImageGenerationTask = Task { [weak self] in
            self?.generateThumbnailImage()
        }
    }

    @MainActor
    func ensurePreviewImage() {
        guard image != nil else { return }
        guard previewImage == nil else { return }
        guard previewImageGenerationTask == nil else { return }
        previewImageGenerationTask = Task { [weak self] in
            self?.generatePreviewImage()
        }
    }

    @MainActor
    func asyncGetPreviewImage() async -> NSImage? {
        if let image = previewImage {
            return image
        }
        ensurePreviewImage()
        _ = await previewImageGenerationTask?.result
        return previewImage
    }

    @MainActor
    func cleanupImages() {
        thumbnailImageGenerationTask?.cancel()
        previewImageGenerationTask?.cancel()
        thumbnailImage?.recache()
        previewImage?.recache()
        thumbnailImage = nil
        previewImage = nil
    }

    @MainActor
    private func generateThumbnailImage() {
        guard let image else { return }
        thumbnailImage = image.resized(to: HistoryItemDecorator.thumbnailImageSize)
    }

    @MainActor
    private func generatePreviewImage() {
        guard let image else { return }
        previewImage = image.resized(to: HistoryItemDecorator.previewImageSize)
    }

    @MainActor
    func sizeImages() {
        generatePreviewImage()
        generateThumbnailImage()
    }

    func highlight(_ query: String, _ ranges: [MatchRange]) {
        guard !query.isEmpty, !title.isEmpty else {
            attributedTitle = nil
            return
        }

        var attributedString = AttributedString(title.shortened(to: 500))
        // Search ranges use Unicode scalar offsets in the full title, while
        // AttributedString indexes use grapheme-cluster characters. Build the
        // mapping from the displayed (possibly shortened) title before indexing.
        var scalarBoundaries = [0]
        var scalarCount = 0
        for character in attributedString.characters {
            scalarCount += character.unicodeScalars.count
            scalarBoundaries.append(scalarCount)
        }
        let characterCount = scalarBoundaries.count - 1

        for range in ranges {
            guard let start = Int(exactly: range.start),
                  let end = Int(exactly: range.end),
                  start >= 0, end > start, start < scalarCount
            else {
                continue
            }

            let visibleEnd = min(end, scalarCount)
            let lowerOffset = (scalarBoundaries.firstIndex { $0 > start } ?? characterCount) - 1
            let upperOffset = scalarBoundaries.firstIndex { $0 >= visibleEnd } ?? characterCount
            guard lowerOffset >= 0, lowerOffset < upperOffset else {
                continue
            }

            let lower = attributedString.index(attributedString.startIndex, offsetByCharacters: lowerOffset)
            let upper = attributedString.index(attributedString.startIndex, offsetByCharacters: upperOffset)
            switch Defaults[.highlightMatch] {
            case .bold:
                attributedString[lower ..< upper].font = .bold(.body)()
            case .italic:
                attributedString[lower ..< upper].font = .italic(.body)()
            case .underline:
                attributedString[lower ..< upper].underlineStyle = .single
            default:
                attributedString[lower ..< upper].backgroundColor = .findHighlightColor
                attributedString[lower ..< upper].foregroundColor = .black
            }
        }

        attributedTitle = attributedString
    }
}
