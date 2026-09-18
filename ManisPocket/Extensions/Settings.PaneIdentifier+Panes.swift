import Settings

enum SettingsLayout {
    static let contentWidth = 550.0
    static let paneWidth = contentWidth + 60
}

extension Settings.PaneIdentifier {
    static let advanced = Self("advanced")
    static let appearance = Self("appearance")
    static let general = Self("general")
    static let ignore = Self("ignore")
    static let pins = Self("pins")
    static let storage = Self("storage")
}
