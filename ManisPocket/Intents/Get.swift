import AppIntents
import AppKit
import Foundation

struct Get: AppIntent, CustomIntentMigratedAppIntent {
    static let intentClassName = "GetIntent"

    static var title: LocalizedStringResource = "Get Item from Clipboard History"
    static var description = IntentDescription("""
    Gets an item from ManisPocket clipboard history.
    The returned item can be used to access its plain/rich/HTML text, image contents or file location.
    """)

    @Parameter(title: "Selected", default: true)
    var selected: Bool

    @Parameter(title: "Number", default: 1)
    var number: Int

    private let positionOffset = 1

    static var parameterSummary: some ParameterSummary {
        When(\.$selected, .equalTo, false) {
            Summary {
                \.$number
                \.$selected
            }
        } otherwise: {
            Summary {
                \.$selected
            }
        }
    }

    func perform() async throws -> some IntentResult & ReturnsValue<HistoryItemAppEntity> {
        var item: ClipboardItem?
        if selected {
            item = AppState.shared.navigator.selection.first?.item
        } else {
            let index = number - positionOffset
            if AppState.shared.history.items.count >= index {
                item = AppState.shared.history.items[index].item
            }
        }

        guard let item else {
            throw AppIntentError.notFound
        }

        let intentItem = HistoryItemAppEntity()
        intentItem.text = Clipboard.shared.getText(from: item)

        if let htmlContent = item.contents.first(where: { $0.contentType == NSPasteboard.PasteboardType.html.rawValue }),
           let value = htmlContent.value
        {
            intentItem.html = String(data: Data(value), encoding: .utf8)
        }

        if let fileContent = item.contents.first(where: { $0.contentType == NSPasteboard.PasteboardType.fileURL.rawValue }),
           let value = fileContent.value,
           let url = URL(dataRepresentation: Data(value), relativeTo: nil, isAbsolute: true)
        {
            intentItem.file = url
        }

        if let imageData = Clipboard.shared.getImageData(from: item) {
            let file = URL.documentsDirectory.appending(path: "image.png")
            try imageData.write(to: file, options: [.atomic, .completeFileProtection])
            intentItem.image = file
        }

        if let rtfContent = item.contents.first(where: { $0.contentType == NSPasteboard.PasteboardType.rtf.rawValue }),
           let value = rtfContent.value
        {
            intentItem.richText = String(data: Data(value), encoding: .utf8)
        }

        return .result(value: intentItem)
    }
}
