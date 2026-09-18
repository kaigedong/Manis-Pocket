import AppKit
import Defaults

// swiftlint:disable identifier_name
// swiftlint:disable type_name
class Sorter {
    enum By: String, CaseIterable, Identifiable, CustomStringConvertible, Defaults.Serializable {
        case lastCopiedAt
        case firstCopiedAt
        case numberOfCopies

        var id: Self {
            self
        }

        var description: String {
            switch self {
            case .lastCopiedAt:
                NSLocalizedString("LastCopiedAt", tableName: "StorageSettings", comment: "")
            case .firstCopiedAt:
                NSLocalizedString("FirstCopiedAt", tableName: "StorageSettings", comment: "")
            case .numberOfCopies:
                NSLocalizedString("NumberOfCopies", tableName: "StorageSettings", comment: "")
            }
        }
    }

    func sort(_ items: [HistoryItem], by: By = Defaults[.sortBy]) -> [HistoryItem] {
        items
            .sorted(by: { bySortingAlgorithm($0, $1, by) })
            .sorted(by: byPinned)
    }

    private func bySortingAlgorithm(_ lhs: HistoryItem, _ rhs: HistoryItem, _ by: By) -> Bool {
        switch by {
        case .firstCopiedAt:
            lhs.firstCopiedAt > rhs.firstCopiedAt
        case .numberOfCopies:
            lhs.numberOfCopies > rhs.numberOfCopies
        default:
            lhs.lastCopiedAt > rhs.lastCopiedAt
        }
    }

    private func byPinned(_ lhs: HistoryItem, _ rhs: HistoryItem) -> Bool {
        if Defaults[.pinTo] == .bottom {
            (lhs.pin == nil) && (rhs.pin != nil)
        } else {
            (lhs.pin != nil) && (rhs.pin == nil)
        }
    }
}

// swiftlint:enable identifier_name
// swiftlint:enable type_name
