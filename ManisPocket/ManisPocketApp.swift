import SwiftUI

@main
enum ManisPocketEntryPoint {
    @MainActor
    static func main() {
        // dyld must load every linked framework before reaching this entry point.
        // Do not initialize the app, database, clipboard or sync in packaging checks.
        if CommandLine.arguments.contains("--startup-smoke-test") {
            print("Manis Pocket startup smoke test passed")
            return
        }
        ManisPocketApp.main()
    }
}

struct ManisPocketApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate

    /// It's impossible to create sceneless application,
    /// so we are hacking this around by creating a menubar
    /// scene that is always hidden.
    @State private var hiddenMenu: Bool = false

    var body: some Scene {
        MenuBarExtra("", isInserted: $hiddenMenu) {
            EmptyView()
        }
    }
}
