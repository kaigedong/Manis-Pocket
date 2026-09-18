import Defaults
@testable import ManisPocket
import XCTest

// swiftlint:disable type_body_length
class ClipboardTests: XCTestCase {
    let clipboard = Clipboard.shared
    let pasteboard = NSPasteboard.general
    let image = NSImage(named: "NSInfo")!
    let coloredString = NSAttributedString(string: "foo",
                                           attributes: [.foregroundColor: NSColor.red])

    let dynamicType = NSPasteboard.PasteboardType(rawValue: "dyn.ah62d4qmxhk4d425try1g44pdsm11g55gsu1e82xnqzv")
    let customType = NSPasteboard.PasteboardType(rawValue: "org.manispocket.ConfidentialType")
    let fileURLType = NSPasteboard.PasteboardType.fileURL
    let htmlType = NSPasteboard.PasteboardType.html
    let rtfType = NSPasteboard.PasteboardType.rtf
    let stringType = NSPasteboard.PasteboardType.string
    let tiffType = NSPasteboard.PasteboardType.tiff
    let transientType = NSPasteboard.PasteboardType.transient
    let unknownType = NSPasteboard.PasteboardType(rawValue: "com.apple.AnnotationKit.AnnotationItem")

    let savedEnabledTypes = Defaults[.enabledPasteboardTypes]
    let savedIgnoreEvents = Defaults[.ignoreEvents]
    let savedIgnoreAllAppsExceptListed = Defaults[.ignoreAllAppsExceptListed]
    let savedIgnoredApps = Defaults[.ignoredApps]
    let savedIgnoredPasteboardTypes = Defaults[.ignoredPasteboardTypes]

    override func setUp() {
        super.setUp()
        Defaults[.ignoreAllAppsExceptListed] = false
        Defaults[.ignoreEvents] = false
    }

    override func tearDown() {
        super.tearDown()
        Defaults[.enabledPasteboardTypes] = savedEnabledTypes
        Defaults[.ignoreEvents] = savedIgnoreEvents
        Defaults[.ignoreOnlyNextEvent] = false
        Defaults[.ignoreAllAppsExceptListed] = savedIgnoreAllAppsExceptListed
        Defaults[.ignoredApps] = savedIgnoredApps
        Defaults[.ignoredPasteboardTypes] = savedIgnoredPasteboardTypes
        clipboard.clearHooks()
    }

    func testChangesListenerAndAddHooks() {
        let hookExpectation = expectation(description: "Hook is called")
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([.string], owner: nil)
        pasteboard.setString("bar", forType: .string)
        waitForExpectations(timeout: 2)
    }

    func testIgnoreStringWithOnlySpaces() {
        let hookExpectation = expectation(description: "Hook is called")
        hookExpectation.isInverted = true
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([.string], owner: nil)
        pasteboard.setString(" ", forType: .string)
        waitForExpectations(timeout: 2)
    }

    func testIgnoreStringWithOnlyNewlines() {
        let hookExpectation = expectation(description: "Hook is called")
        hookExpectation.isInverted = true
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([.string], owner: nil)
        pasteboard.setString("\n", forType: .string)
        waitForExpectations(timeout: 2)
    }

    func testDoesNotIgnoreRTF() {
        let hookExpectation = expectation(description: "Hook is called")
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        let rtf = NSAttributedString(string: "foo").rtf(
            from: NSRange(0 ... 2),
            documentAttributes: [:]
        )
        pasteboard.declareTypes([.rtf], owner: nil)
        pasteboard.setData(rtf, forType: .rtf)
        waitForExpectations(timeout: 2)
    }

    func testDoesNotIgnoreHTML() {
        let hookExpectation = expectation(description: "Hook is called")
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([.html], owner: nil)
        pasteboard.setString("foo", forType: .html)
        waitForExpectations(timeout: 2)
    }

    func testIgnoreEventsIsEnabled() {
        Defaults[.ignoreEvents] = true

        let hookExpectation = expectation(description: "Hook is called")
        hookExpectation.isInverted = true
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([.string], owner: nil)
        pasteboard.setString("foo", forType: .string)
        waitForExpectations(timeout: 2)
    }

    func testIgnoreOnlyNextEventIsEnabled() {
        Defaults[.ignoreEvents] = true
        Defaults[.ignoreOnlyNextEvent] = true

        let hookExpectation = expectation(description: "Hook is called")
        hookExpectation.isInverted = true
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([.string], owner: nil)
        pasteboard.setString("foo", forType: .string)
        waitForExpectations(timeout: 2)

        XCTAssertFalse(Defaults[.ignoreEvents])
        XCTAssertFalse(Defaults[.ignoreOnlyNextEvent])
    }

    func testIgnoreApplication() throws {
        let frontmostApp = try XCTUnwrap(NSWorkspace.shared.frontmostApplication?.bundleIdentifier)
        Defaults[.ignoredApps] = [frontmostApp]

        let hookExpectation = expectation(description: "Hook is called")
        hookExpectation.isInverted = true
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([.string], owner: nil)
        pasteboard.setString("bar", forType: .string)
        waitForExpectations(timeout: 2)
    }

    func testIgnoreAllApplicationsExcept() throws {
        let frontmostApp = try XCTUnwrap(NSWorkspace.shared.frontmostApplication?.bundleIdentifier)
        Defaults[.ignoreAllAppsExceptListed] = true
        Defaults[.ignoredApps] = [frontmostApp]

        let hookExpectation = expectation(description: "Hook is called")
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([.string], owner: nil)
        pasteboard.setString("bar", forType: .string)
        waitForExpectations(timeout: 2)
    }

    func testIgnoreTransientTypes() {
        let hookExpectation = expectation(description: "Hook is called")
        hookExpectation.isInverted = true
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([.string, transientType], owner: nil)
        pasteboard.setString("bar", forType: .string)
        waitForExpectations(timeout: 2)
    }

    func testIgnoreCustomTypes() {
        Defaults[.ignoredPasteboardTypes] = [customType.rawValue]

        let hookExpectation = expectation(description: "Hook is called")
        hookExpectation.isInverted = true
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([.string, customType], owner: nil)
        pasteboard.setString("bar", forType: .string)
        waitForExpectations(timeout: 2)
    }

    func testIgnoreCopiesWithUnknownTypes() {
        let hookExpectation = expectation(description: "Hook is called")
        hookExpectation.isInverted = true
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([unknownType], owner: nil)
        pasteboard.setString(" ", forType: unknownType)
        waitForExpectations(timeout: 2)
    }

    @MainActor
    func testCopy() throws {
        let imageData = try XCTUnwrap(image.tiffRepresentation)
        let contents = try [
            ClipboardContent(contentType: stringType.rawValue, value: XCTUnwrap("foo".data(using: .utf8))),
            ClipboardContent(contentType: tiffType.rawValue, value: imageData),
            ClipboardContent(contentType: fileURLType.rawValue, value: XCTUnwrap("file://foo.bar".data(using: .utf8))),
        ]
        let item = clipboardItem(contents: contents, application: "com.foo.bar")
        clipboard.copy(item)
        XCTAssertEqual(pasteboard.string(forType: .string), "foo")
        XCTAssertEqual(pasteboard.data(forType: .tiff), imageData)
        XCTAssertEqual(pasteboard.string(forType: .fileURL), "file://foo.bar")
        XCTAssertEqual(pasteboard.string(forType: .fromManisPocket), "")
        XCTAssertEqual(pasteboard.string(forType: .source), "com.foo.bar")
    }

    @MainActor
    func testCopyWithoutFormatting() throws {
        let contents = try [
            ClipboardContent(contentType: stringType.rawValue, value: XCTUnwrap("foo".data(using: .utf8))),
            ClipboardContent(contentType: fileURLType.rawValue, value: XCTUnwrap("file://foo.bar".data(using: .utf8))),
            ClipboardContent(contentType: rtfType.rawValue,
                             value: coloredString.rtf(from: NSRange(location: 0, length: coloredString.length),
                                                      documentAttributes: [:])),
        ]
        let item = clipboardItem(contents: contents, application: "com.foo.bar")
        clipboard.copy(item, removeFormatting: true)
        XCTAssertEqual(pasteboard.string(forType: .string), "foo")
        XCTAssertEqual(pasteboard.string(forType: .fromManisPocket), "")
        XCTAssertEqual(pasteboard.string(forType: .source), "com.foo.bar")
        XCTAssertEqual(pasteboard.string(forType: .fileURL), "file://foo.bar")
        XCTAssertNil(pasteboard.data(forType: .rtf))
    }

    func testHandlesItemsWithoutData() {
        let hookExpectation = expectation(description: "Hook is called")
        pasteboard.clearContents()
        clipboard.onNewCopy { (_: ClipboardItem, _: Bool) in
            hookExpectation.fulfill()
        }
        clipboard.start()
        pasteboard.declareTypes([.fileURL, .string], owner: nil)
        // fileURL is left without data
        pasteboard.setString("bar", forType: .string)
        waitForExpectations(timeout: 2)
    }

    func testMergesMultipleItems() throws {
        let hookExpectation = expectation(description: "Hook is called")
        clipboard.onNewCopy { (item: ClipboardItem, _: Bool) in
            XCTAssertEqual(
                Set(item.contents.map(\.contentType)),
                Set([self.tiffType.rawValue, self.stringType.rawValue])
            )
            hookExpectation.fulfill()
        }

        let item1 = NSPasteboardItem()
        item1.setString("foo", forType: .string)
        let item2 = NSPasteboardItem()
        try item2.setData(XCTUnwrap(image.tiffRepresentation), forType: .tiff)

        clipboard.start()
        pasteboard.clearContents()
        pasteboard.writeObjects([item1, item2])

        waitForExpectations(timeout: 2)
    }

    func testRemovesDisabledTypes() throws {
        Defaults[.enabledPasteboardTypes] = [.fileURL]

        let hookExpectation = expectation(description: "Hook is called")
        clipboard.onNewCopy { (item: ClipboardItem, _: Bool) in
            XCTAssertEqual(item.contents.map(\.contentType), [self.fileURLType.rawValue])
            hookExpectation.fulfill()
        }

        let item = NSPasteboardItem()
        item.setString("foo", forType: .string)
        try item.setData(XCTUnwrap(image.tiffRepresentation), forType: .tiff)
        try item.setData(XCTUnwrap("file://foo.bar".data(using: .utf8)), forType: .fileURL)

        clipboard.start()
        pasteboard.clearContents()
        pasteboard.writeObjects([item])

        waitForExpectations(timeout: 2)
    }

    func testRemovesDynamicTypes() throws {
        let hookExpectation = expectation(description: "Hook is called")
        clipboard.onNewCopy { (item: ClipboardItem, _: Bool) in
            XCTAssertEqual(item.contents.map(\.contentType), [self.stringType.rawValue])
            hookExpectation.fulfill()
        }

        let item = NSPasteboardItem()
        item.setString("foo", forType: .string)
        try item.setData(XCTUnwrap("".data(using: .utf8)), forType: dynamicType)

        clipboard.start()
        pasteboard.clearContents()
        pasteboard.writeObjects([item])

        waitForExpectations(timeout: 2)
    }

    private func clipboardItem(contents: [ClipboardContent], application: String? = nil) -> ClipboardItem {
        ClipboardItem(
            id: UUID().uuidString,
            application: application,
            firstCopiedAt: 0,
            lastCopiedAt: 0,
            numberOfCopies: 1,
            pin: nil,
            title: "",
            contents: contents,
            syncTimestamp: 0,
            syncSource: nil,
            syncDeleted: false
        )
    }
}

// swiftlint:enable type_body_length
