import Defaults
@testable import Maccy
import XCTest

@MainActor
class HistoryItemDecoratorTests: XCTestCase {
    let boldFont = NSFont.boldSystemFont(ofSize: NSFont.systemFontSize)
    let savedHighlightMatch = Defaults[.highlightMatch]
    let savedImageMaxHeight = Defaults[.imageMaxHeight]

    var firstCopiedAt: Date! {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy/MM/dd HH:mm:ss"
        return formatter.date(from: "2020/07/10 12:31:34")
    }

    var lastCopiedAt: Date! {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy/MM/dd HH:mm:ss"
        return formatter.date(from: "2020/07/10 12:41:34")
    }

    var firstCopiedAtMilliseconds: Int64 {
        Int64(firstCopiedAt.timeIntervalSince1970 * 1000)
    }

    var lastCopiedAtMilliseconds: Int64 {
        Int64(lastCopiedAt.timeIntervalSince1970 * 1000)
    }

    override func setUp() {
        super.setUp()
        Defaults[.highlightMatch] = .bold
        Defaults[.imageMaxHeight] = 40
    }

    override func tearDown() {
        super.tearDown()
        Defaults[.imageMaxHeight] = savedImageMaxHeight
        Defaults[.highlightMatch] = savedHighlightMatch
    }

    func testString() {
        let title = "foo"
        let itemDecorator = historyItemDecorator(title)
        XCTAssertEqual(itemDecorator.title, title)
        XCTAssertNil(itemDecorator.previewImage)
        XCTAssertNil(itemDecorator.thumbnailImage)
    }

    func testRTF() {
        let rtf = NSAttributedString(string: "foo").rtf(
            from: NSRange(0 ... 2),
            documentAttributes: [:]
        )
        let itemDecorator = historyItemDecorator(rtf, .rtf)
        XCTAssertEqual(itemDecorator.title, "foo")
        XCTAssertNil(itemDecorator.previewImage)
        XCTAssertNil(itemDecorator.thumbnailImage)
    }

    func testHTML() {
        let html = "<a href='#'>foo</a>".data(using: .utf8)
        let itemDecorator = historyItemDecorator(html, .html)
        XCTAssertEqual(itemDecorator.title, "foo")
        XCTAssertNil(itemDecorator.previewImage)
        XCTAssertNil(itemDecorator.thumbnailImage)
    }

    func testImage() throws {
        let image = try XCTUnwrap(NSImage(named: "StatusBarMenuImage"))
        let itemDecorator = historyItemDecorator(image)
        itemDecorator.sizeImages()
        XCTAssertEqual(itemDecorator.title, "")
        XCTAssertEqual(itemDecorator.previewImage?.size, image.size)
        XCTAssertEqual(itemDecorator.thumbnailImage?.size, image.size)
    }

    /// We also need to add test for image with width bigger than max width.
    func testImageWithHeightBiggerThanMaxHeight() throws {
        let image = try XCTUnwrap(NSImage(named: "NSApplicationIcon"))
        let itemDecorator = historyItemDecorator(image)
        itemDecorator.sizeImages()
        XCTAssertEqual(itemDecorator.thumbnailImage?.size, NSSize(width: 40, height: 40))
    }

    func testFile() {
        let url = URL(fileURLWithPath: "/tmp/foo.bar")
        let itemDecorator = historyItemDecorator(url)
        XCTAssertEqual(itemDecorator.title, "file:///tmp/foo.bar")
        XCTAssertNil(itemDecorator.previewImage)
        XCTAssertNil(itemDecorator.thumbnailImage)
    }

    func testFileWithEscapedChars() {
        let url = URL(fileURLWithPath: "/tmp/产品培训/产品培训.txt")
        let itemDecorator = historyItemDecorator(url)
        XCTAssertEqual(itemDecorator.title, "file:///tmp/产品培训/产品培训.txt")
        XCTAssertNil(itemDecorator.previewImage)
        XCTAssertNil(itemDecorator.thumbnailImage)
    }

    func testItemWithoutData() {
        let itemDecorator = historyItemDecorator(nil)
        XCTAssertEqual(itemDecorator.title, "")
        XCTAssertNil(itemDecorator.previewImage)
        XCTAssertNil(itemDecorator.thumbnailImage)
    }

    func testUnpinnedByDefault() {
        let itemDecorator = historyItemDecorator("foo")
        XCTAssertNil(itemDecorator.item.pin)
        XCTAssertFalse(itemDecorator.isPinned)
    }

    func testPin() {
        let itemDecorator = historyItemDecorator("foo")
        itemDecorator.item.pin = "b"
        XCTAssertNotNil(itemDecorator.item.pin)
        XCTAssertTrue(itemDecorator.isPinned)
    }

    func testUnpin() {
        let itemDecorator = historyItemDecorator("foo")
        itemDecorator.item.pin = "b"
        itemDecorator.item.pin = nil
        XCTAssertNil(itemDecorator.item.pin)
        XCTAssertFalse(itemDecorator.isPinned)
    }

    func testHighlight() throws {
        let itemDecorator = historyItemDecorator("foo bar baz")
        itemDecorator.highlight("random", [
            range(from: 1, to: 2),
            range(from: 8, to: 10),
        ])
        var expectedTitle = AttributedString("foo bar baz")
        try expectedTitle[XCTUnwrap(expectedTitle.range(of: "oo"))].font = .bold(.body)()
        try expectedTitle[XCTUnwrap(expectedTitle.range(of: "baz"))].font = .bold(.body)()
        XCTAssertEqual(itemDecorator.attributedTitle, expectedTitle)
        itemDecorator.highlight("", [])
        XCTAssertEqual(itemDecorator.attributedTitle, nil)
    }

    func testHighlightSkipsMatchBeyondDisplayedTitle() {
        let itemDecorator = historyItemDecorator(String(repeating: "a", count: 600) + "z")

        itemDecorator.highlight("z", [MatchRange(start: 600, end: 601)])

        XCTAssertEqual(itemDecorator.attributedTitle, AttributedString(itemDecorator.title.shortened(to: 500)))
    }

    func testHighlightMapsUnicodeScalarRangesToCharacters() throws {
        let itemDecorator = historyItemDecorator("e\u{301}x")

        itemDecorator.highlight("x", [MatchRange(start: 2, end: 3)])

        var expectedTitle = AttributedString("e\u{301}x")
        try expectedTitle[XCTUnwrap(expectedTitle.range(of: "x"))].font = .bold(.body)()
        XCTAssertEqual(itemDecorator.attributedTitle, expectedTitle)
    }

    private func historyItemDecorator(
        _ value: String?,
        application: String? = "com.apple.finder"
    ) -> HistoryItemDecorator {
        let contents = [
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.string.rawValue,
                value: value?.data(using: .utf8)
            ),
        ]
        var item = clipboardItem(contents: contents, application: application)
        item.title = Clipboard.shared.generateTitle(for: item)

        return HistoryItemDecorator(item)
    }

    private func historyItemDecorator(
        _ value: Data?,
        _ type: NSPasteboard.PasteboardType
    ) -> HistoryItemDecorator {
        let contents = [
            ClipboardContent(
                contentType: type.rawValue,
                value: value
            ),
        ]
        var item = clipboardItem(contents: contents, numberOfCopies: 2)
        item.title = Clipboard.shared.generateTitle(for: item)

        return HistoryItemDecorator(item)
    }

    private func historyItemDecorator(_ value: NSImage) -> HistoryItemDecorator {
        let contents = [
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.tiff.rawValue,
                value: value.tiffRepresentation!
            ),
        ]
        var item = clipboardItem(contents: contents, numberOfCopies: 2)
        item.title = Clipboard.shared.generateTitle(for: item)

        return HistoryItemDecorator(item)
    }

    private func historyItemDecorator(_ value: URL) -> HistoryItemDecorator {
        let contents = [
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.fileURL.rawValue,
                value: value.dataRepresentation
            ),
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.string.rawValue,
                value: value.lastPathComponent.data(using: .utf8)
            ),
        ]
        var item = clipboardItem(contents: contents, numberOfCopies: 2)
        item.title = Clipboard.shared.generateTitle(for: item)

        return HistoryItemDecorator(item)
    }

    private func clipboardItem(
        contents: [ClipboardContent],
        application: String? = "com.apple.finder",
        numberOfCopies: Int32 = 1
    ) -> ClipboardItem {
        ClipboardItem(
            id: UUID().uuidString,
            application: application,
            firstCopiedAt: firstCopiedAtMilliseconds,
            lastCopiedAt: lastCopiedAtMilliseconds,
            numberOfCopies: numberOfCopies,
            pin: nil,
            title: "",
            contents: contents,
            syncTimestamp: lastCopiedAtMilliseconds,
            syncSource: nil,
            syncDeleted: false
        )
    }

    private func range(from: Int64, to: Int64) -> MatchRange {
        MatchRange(start: from, end: to + 1)
    }
}
