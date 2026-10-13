import AppKit
@testable import ManisPocket
import Settings
import XCTest

@MainActor
class SettingsTests: XCTestCase {
    override func tearDown() {
        settingsWindow?.close()
        super.tearDown()
    }

    private var settingsWindow: NSWindow? {
        NSApp.windows.first { $0.windowController is SettingsWindowController }
    }

    func testOpenSyncActivatesContentAndToolbar() throws {
        AppState.shared.openPreferences(pane: .general)
        let window = try XCTUnwrap(settingsWindow)
        let generalView = try XCTUnwrap(window.contentViewController?.view.subviews.first)

        AppState.shared.openPreferencesToSync()

        XCTAssertEqual(window.toolbar?.selectedItemIdentifier?.rawValue, "sync")
        XCTAssertEqual(window.title, NSLocalizedString("Title", tableName: "SyncSettings", comment: ""))
        XCTAssertNil(generalView.superview, "The general pane must be replaced, not just deselected in the toolbar")
        XCTAssertFalse(try XCTUnwrap(window.contentViewController?.view.subviews).isEmpty)
    }

    func testReopenPreferencesPreservesSyncPane() throws {
        AppState.shared.openPreferencesToSync()
        let window = try XCTUnwrap(settingsWindow)
        window.close()

        AppState.shared.openPreferences()

        XCTAssertTrue(window.isVisible)
        XCTAssertEqual(window.toolbar?.selectedItemIdentifier?.rawValue, "sync")
        XCTAssertEqual(window.title, NSLocalizedString("Title", tableName: "SyncSettings", comment: ""))
    }

    func testPeerUpdatesDoNotReopenClosedSettings() async throws {
        AppState.shared.openPreferences(pane: .general)
        let window = try XCTUnwrap(settingsWindow)
        window.close()

        await simulatePeerUpdates()

        XCTAssertFalse(window.isVisible)
    }

    func testPeerUpdatesDoNotSwitchVisibleSettings() async throws {
        AppState.shared.openPreferences(pane: .general)
        let window = try XCTUnwrap(settingsWindow)

        await simulatePeerUpdates()

        XCTAssertEqual(window.toolbar?.selectedItemIdentifier?.rawValue, "general")
        XCTAssertEqual(window.title, NSLocalizedString("Title", tableName: "GeneralSettings", comment: ""))
    }

    private func simulatePeerUpdates() async {
        let updates = expectation(description: "Discovery, repeated connection and reconnection still update peers")
        updates.expectedFulfillmentCount = 4
        let notifications = expectation(description: "Connected peers still produce connection notifications")
        notifications.expectedFulfillmentCount = 3
        var settingsRequests = 0
        var receivedStates: [Bool] = []
        let center = NotificationCenter.default
        let legacyToken = center.addObserver(
            forName: Notification.Name("showSyncSettings"), object: nil, queue: .main
        ) { _ in
            settingsRequests += 1
        }
        let updatesToken = center.addObserver(forName: .syncPeerDiscovered, object: nil, queue: .main) { event in
            XCTAssertEqual(event.userInfo?["peerID"] as? String, "settings-test-peer")
            XCTAssertEqual(event.userInfo?["displayName"] as? String, "Test device")
            XCTAssertEqual(event.userInfo?["addresses"] as? [String], ["127.0.0.1:31774"])
            if let connected = event.userInfo?["isConnected"] as? Bool {
                receivedStates.append(connected)
            }
            updates.fulfill()
        }
        defer {
            center.removeObserver(legacyToken)
            center.removeObserver(updatesToken)
        }
        // Avoid system notifications and permission prompts during the test.
        let observer = ManisPocketSyncObserver { name in
            XCTAssertEqual(name, "Test device")
            notifications.fulfill()
        }
        for connected in [true, true, false, true] {
            observer.onPeerDiscovered(
                peerId: "settings-test-peer", displayName: "Test device",
                addresses: ["127.0.0.1:31774"], isConnected: connected
            )
        }
        await fulfillment(of: [updates, notifications], timeout: 2)
        XCTAssertEqual(receivedStates, [true, true, false, true])
        XCTAssertEqual(settingsRequests, 0, "Background peer updates must never request settings")
    }
}
