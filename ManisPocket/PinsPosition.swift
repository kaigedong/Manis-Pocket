import Defaults
import Foundation

enum PinsPosition: String, CaseIterable, Identifiable, CustomStringConvertible, Defaults.Serializable {
    case top
    case bottom

    var id: Self {
        self
    }

    var description: String {
        switch self {
        case .top:
            NSLocalizedString("PinToTop", tableName: "AppearanceSettings", comment: "")
        case .bottom:
            NSLocalizedString("PinToBottom", tableName: "AppearanceSettings", comment: "")
        }
    }
}
