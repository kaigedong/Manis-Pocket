import Defaults
import Foundation

enum HighlightMatch: String, CaseIterable, Identifiable, CustomStringConvertible, Defaults.Serializable {
    case color
    case bold
    case italic
    case underline

    var id: Self {
        self
    }

    var description: String {
        switch self {
        case .bold:
            NSLocalizedString("HighlightMatchBold", tableName: "AppearanceSettings", comment: "")
        case .color:
            NSLocalizedString("HighlightMatchColor", tableName: "AppearanceSettings", comment: "")
        case .italic:
            NSLocalizedString("HighlightMatchItalic", tableName: "AppearanceSettings", comment: "")
        case .underline:
            NSLocalizedString("HighlightMatchUnderline", tableName: "AppearanceSettings", comment: "")
        }
    }
}
