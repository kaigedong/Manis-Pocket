import Defaults
import Sparkle

@Observable
class SoftwareUpdater: NSObject, SPUUpdaterDelegate {
    var automaticallyChecksForUpdates = false {
        didSet {
            updater.automaticallyChecksForUpdates = automaticallyChecksForUpdates
        }
    }

    private var updater: SPUUpdater!
    private var automaticallyChecksForUpdatesObservation: NSKeyValueObservation?

    private var updaterController: SPUStandardUpdaterController!

    override init() {
        super.init()
        updaterController = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: self,
            userDriverDelegate: nil
        )
        updater = updaterController.updater
        automaticallyChecksForUpdatesObservation = updater.observe(
            \.automaticallyChecksForUpdates,
            options: [.initial, .new, .old]
        ) { [unowned self] updater, change in
            guard change.newValue != change.oldValue else {
                return
            }

            automaticallyChecksForUpdates = updater.automaticallyChecksForUpdates
        }
    }

    func checkForUpdates() {
        updater.checkForUpdates()
    }

    // MARK: - SPUUpdaterDelegate

    func feedURLString(for _: SPUUpdater) -> String? {
        if Defaults[.betaUpdates] {
            return "https://github.com/kaigedong/Maccy-plus/releases/download/latest-beta/appcast-beta.xml"
        }
        // Return nil to use the default SUFeedURL from Info.plist
        return nil
    }
}
