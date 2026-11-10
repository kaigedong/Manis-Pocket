import AppKit
import Defaults

enum MenuIcon: String, CaseIterable, Identifiable, Defaults.Serializable {
    case manispocket
    case clipboard
    case scissors
    case paperclip

    var id: Self {
        self
    }

    var image: NSImage {
        switch self {
        case .manispocket:
            NSImage(systemSymbolName: "doc.on.clipboard", accessibilityDescription: "Manis Pocket")!
        case .clipboard:
            NSImage(named: .clipboard)!
        case .scissors:
            NSImage(named: .scissors)!
        case .paperclip:
            NSImage(named: .paperclip)!
        }
    }
}
