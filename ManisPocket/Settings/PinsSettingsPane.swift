import Defaults
import SwiftUI

struct PinsSettingsPane: View {
    @Environment(AppState.self) private var appState

    @State private var availablePins: [String] = []
    @State private var selection: HistoryItemDecorator.ID?

    private var pinnedItems: [HistoryItemDecorator] {
        appState.history.pinnedItems
    }

    var body: some View {
        VStack(alignment: .leading) {
            Table(pinnedItems, selection: $selection) {
                TableColumn(Text("Key", tableName: "PinsSettings")) { decorator in
                    if let pin = decorator.item.pin {
                        Text(pin)
                    }
                }
                .width(60)

                TableColumn(Text("Alias", tableName: "PinsSettings")) { decorator in
                    Text(decorator.title)
                }

                TableColumn(Text("Content", tableName: "PinsSettings")) { decorator in
                    Text(Clipboard.shared.getText(from: decorator.item) ?? "")
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
            .onAppear {
                availablePins = History.availablePins
            }
            .onDeleteCommand {
                guard let selection else { return }
                if let item = appState.history.items.first(where: { $0.id == selection }) {
                    appState.history.delete(item)
                }
            }

            Text("PinCustomizationDescription", tableName: "PinsSettings")
                .foregroundStyle(.gray)
                .controlSize(.small)
        }
        .padding()
        .frame(width: SettingsLayout.paneWidth)
        .frame(minHeight: 400)
    }
}

#Preview {
    PinsSettingsPane()
        .environment(\.locale, .init(identifier: "en"))
}
