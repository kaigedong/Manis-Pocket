import AppKit.NSEvent
import Defaults

enum HistoryItemAction {
    case unknown
    case copy
    case paste
    case pasteWithoutFormatting

    init(_ modifierFlags: NSEvent.ModifierFlags) { // swiftlint:disable:this cyclomatic_complexity
        switch modifierFlags {
        case .command where !Defaults[.pasteByDefault]:
            self = .copy
        case .command where Defaults[.pasteByDefault] && !Defaults[.removeFormattingByDefault]:
            self = .paste
        case .command where Defaults[.pasteByDefault] && Defaults[.removeFormattingByDefault]:
            self = .pasteWithoutFormatting
        case .option where !Defaults[.pasteByDefault] && !Defaults[.removeFormattingByDefault]:
            self = .paste
        case .option where !Defaults[.pasteByDefault] && Defaults[.removeFormattingByDefault]:
            self = .pasteWithoutFormatting
        case .option where Defaults[.pasteByDefault] && !Defaults[.removeFormattingByDefault]:
            self = .copy
        case .option where Defaults[.pasteByDefault] && Defaults[.removeFormattingByDefault]:
            self = .copy
        case [.option, .shift] where !Defaults[.pasteByDefault] && !Defaults[.removeFormattingByDefault]:
            self = .pasteWithoutFormatting
        case [.option, .shift] where !Defaults[.pasteByDefault] && Defaults[.removeFormattingByDefault]:
            self = .paste
        case [.command, .shift] where Defaults[.pasteByDefault] && !Defaults[.removeFormattingByDefault]:
            self = .pasteWithoutFormatting
        case [.command, .shift] where Defaults[.pasteByDefault] && Defaults[.removeFormattingByDefault]:
            self = .paste
        default:
            self = .unknown
        }
    }

    var modifierFlags: NSEvent.ModifierFlags {
        switch self {
        case .copy where !Defaults[.pasteByDefault]:
            .command
        case .paste where Defaults[.pasteByDefault] && !Defaults[.removeFormattingByDefault]:
            .command
        case .pasteWithoutFormatting where Defaults[.pasteByDefault] && Defaults[.removeFormattingByDefault]:
            .command
        case .paste where !Defaults[.pasteByDefault] && !Defaults[.removeFormattingByDefault]:
            .option
        case .pasteWithoutFormatting where !Defaults[.pasteByDefault] && Defaults[.removeFormattingByDefault]:
            .option
        case .copy where Defaults[.pasteByDefault] && !Defaults[.removeFormattingByDefault]:
            .option
        case .copy where Defaults[.pasteByDefault] && Defaults[.removeFormattingByDefault]:
            .option
        case .pasteWithoutFormatting where !Defaults[.pasteByDefault] && !Defaults[.removeFormattingByDefault]:
            [.option, .shift]
        case .paste where !Defaults[.pasteByDefault] && Defaults[.removeFormattingByDefault]:
            [.option, .shift]
        case .pasteWithoutFormatting where Defaults[.pasteByDefault] && !Defaults[.removeFormattingByDefault]:
            [.command, .shift]
        case .paste where Defaults[.pasteByDefault] && Defaults[.removeFormattingByDefault]:
            [.command, .shift]
        default:
            []
        }
    }
}
