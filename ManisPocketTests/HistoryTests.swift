import Defaults
@testable import ManisPocket
import XCTest

@MainActor
class HistoryTests: XCTestCase {
    let savedSize = Defaults[.size]
    let savedSortBy = Defaults[.sortBy]
    let history = History.shared
    private var nextTimestamp = Int64(Date().timeIntervalSince1970 * 1000)

    override func setUp() {
        super.setUp()
        history.clearAll()
        history.selectedApp = nil
        history.excludedDevices = []
        Defaults[.size] = 10
        Defaults[.sortBy] = .firstCopiedAt
    }

    override func tearDown() {
        super.tearDown()
        Defaults[.size] = savedSize
        Defaults[.sortBy] = savedSortBy
    }

    func testDefaultIsEmpty() {
        XCTAssertEqual(history.items, [])
    }

    func testAdding() {
        let first = history.add(historyItem("foo"))
        let second = history.add(historyItem("bar"))
        XCTAssertEqual(history.items, [second, first])
    }

    func testAddingSame() {
        var first = historyItem("foo")
        first.title = "xyz"
        first.application = "iTerm.app"
        first.pin = "f"
        let firstDecorator = history.add(first)

        let secondDecorator = history.add(historyItem("bar"))

        var third = historyItem("foo")
        third.application = "Xcode.app"
        history.add(third)

        XCTAssertEqual(history.items, [firstDecorator, secondDecorator])
        XCTAssertTrue(history.items[0].item.lastCopiedAt > history.items[0].item.firstCopiedAt)
        // TODO: This works in reality but fails in tests?!
        // XCTAssertEqual(history.items[0].item.numberOfCopies, 2)
        XCTAssertEqual(history.items[0].item.pin, "f")
        XCTAssertEqual(history.items[0].item.title, "xyz")
        XCTAssertEqual(history.items[0].item.application, "iTerm.app")
    }

    func testAddingItemThatIsSupersededByExisting() throws {
        let firstContents = try [
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.string.rawValue,
                value: XCTUnwrap("one".data(using: .utf8))
            ),
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.rtf.rawValue,
                value: XCTUnwrap("two".data(using: .utf8))
            ),
        ]
        var firstItem = historyItem(contents: firstContents)
        firstItem.application = "ManisPocket.app"
        history.add(firstItem)

        let secondContents = try [
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.string.rawValue,
                value: XCTUnwrap("one".data(using: .utf8))
            ),
        ]
        var secondItem = historyItem(contents: secondContents)
        secondItem.application = "ManisPocket.app"
        let second = history.add(secondItem)

        XCTAssertEqual(history.items, [second])
        XCTAssertEqual(Set(history.items[0].item.contents), Set(firstContents))
    }

    func testAddingItemWithDifferentModifiedType() throws {
        let firstContents = try [
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.string.rawValue,
                value: XCTUnwrap("one".data(using: .utf8))
            ),
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.modified.rawValue,
                value: XCTUnwrap("1".data(using: .utf8))
            ),
        ]
        let firstItem = historyItem(contents: firstContents)
        history.add(firstItem)

        let secondContents = try [
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.string.rawValue,
                value: XCTUnwrap("one".data(using: .utf8))
            ),
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.modified.rawValue,
                value: XCTUnwrap("2".data(using: .utf8))
            ),
        ]
        let secondItem = historyItem(contents: secondContents)
        let second = history.add(secondItem)

        XCTAssertEqual(history.items, [second])
        XCTAssertEqual(Set(history.items[0].item.contents), Set(firstContents))
    }

    func testAddingItemFromManisPocket() {
        let firstContents = [
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.string.rawValue,
                value: "one".data(using: .utf8)
            ),
        ]
        var first = historyItem(contents: firstContents)
        first.application = "Xcode.app"
        history.add(first)

        let secondContents = [
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.string.rawValue,
                value: "one".data(using: .utf8)
            ),
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.fromManisPocket.rawValue,
                value: "".data(using: .utf8)
            ),
        ]
        var second = historyItem(contents: secondContents)
        second.application = "ManisPocket.app"
        let secondDecorator = history.add(second)

        XCTAssertEqual(history.items, [secondDecorator])
        XCTAssertEqual(history.items[0].item.application, "Xcode.app")
        XCTAssertEqual(Set(history.items[0].item.contents), Set(firstContents))
    }

    func testModifiedAfterCopying() {
        history.add(historyItem("foo"))

        var modifiedItem = historyItem("bar")
        modifiedItem.contents.append(ClipboardContent(
            contentType: NSPasteboard.PasteboardType.modified.rawValue,
            value: String(Clipboard.shared.changeCount).data(using: .utf8)
        ))
        let modifiedItemDecorator = history.add(modifiedItem)

        XCTAssertEqual(history.items, [modifiedItemDecorator])
        XCTAssertEqual(history.items[0].text, "bar")
    }

    func testClearingUnpinned() {
        let pinned = history.add(historyItem("foo"))
        history.togglePin(pinned)
        history.add(historyItem("bar"))
        history.clear()
        XCTAssertEqual(history.items, [pinned])
    }

    func testClearingAll() {
        history.add(historyItem("foo"))
        history.clear()
        XCTAssertEqual(history.items, [])
    }

    func testMaxSize() {
        var items: [HistoryItemDecorator] = []
        for index in 0 ... 10 {
            items.append(history.add(historyItem(String(index))))
        }

        XCTAssertEqual(history.items.count, 10)
        XCTAssertTrue(history.items.contains(items[10]))
        XCTAssertFalse(history.items.contains(items[0]))
    }

    func testMaxSizeIgnoresPinned() {
        var items: [HistoryItemDecorator] = []

        let item = history.add(historyItem("0"))
        items.append(item)
        history.togglePin(item)

        for index in 1 ... 11 {
            items.append(history.add(historyItem(String(index))))
        }

        XCTAssertEqual(history.items.count, 11)
        XCTAssertTrue(history.items.contains(items[10]))
        XCTAssertTrue(history.items.contains(items[0]))
        XCTAssertFalse(history.items.contains(items[1]))
    }

    func testMaxSizeIsChanged() {
        var items: [HistoryItemDecorator] = []
        for index in 0 ... 10 {
            items.append(history.add(historyItem(String(index))))
        }
        Defaults[.size] = 5
        history.add(historyItem("11"))

        XCTAssertEqual(history.items.count, 5)
        XCTAssertTrue(history.items.contains(items[10]))
        XCTAssertFalse(history.items.contains(items[5]))
    }

    func testRemoving() {
        let foo = history.add(historyItem("foo"))
        let bar = history.add(historyItem("bar"))
        history.delete(foo)
        XCTAssertEqual(history.items, [bar])
    }

    func testSelectingAppShowsOnlyItemsFromThatApp() {
        var chrome = historyItem("chrome")
        chrome.application = "com.google.Chrome"
        history.add(chrome)

        var telegram = historyItem("telegram")
        telegram.application = "org.telegram.Telegram"
        let telegramDecorator = history.add(telegram)

        history.toggleAppSelection("org.telegram.Telegram")
        waitForFilterUpdate()

        XCTAssertEqual(history.items, [telegramDecorator])
    }

    func testSelectingSameAppAgainShowsAllItems() {
        var chrome = historyItem("chrome")
        chrome.application = "com.google.Chrome"
        let chromeDecorator = history.add(chrome)

        var telegram = historyItem("telegram")
        telegram.application = "org.telegram.Telegram"
        let telegramDecorator = history.add(telegram)

        history.toggleAppSelection("org.telegram.Telegram")
        waitForFilterUpdate()
        history.toggleAppSelection("org.telegram.Telegram")
        waitForFilterUpdate()

        XCTAssertEqual(history.items, [telegramDecorator, chromeDecorator])
    }

    private func waitForFilterUpdate() {
        RunLoop.main.run(until: Date().addingTimeInterval(0.3))
    }

    private func historyItem(_ value: String) -> ClipboardItem {
        let contents = [
            ClipboardContent(
                contentType: NSPasteboard.PasteboardType.string.rawValue,
                value: value.data(using: .utf8)
            ),
        ]
        return historyItem(contents: contents)
    }

    private func historyItem(contents: [ClipboardContent]) -> ClipboardItem {
        nextTimestamp += 1
        var item = ClipboardItem(
            id: UUID().uuidString,
            application: nil,
            firstCopiedAt: nextTimestamp,
            lastCopiedAt: nextTimestamp,
            numberOfCopies: 1,
            pin: nil,
            title: "",
            contents: contents,
            syncTimestamp: nextTimestamp,
            syncSource: nil,
            syncDeleted: false
        )
        item.title = Clipboard.shared.generateTitle(for: item)
        return item
    }
}
