import SwiftUI

struct SlideoutContentView: View {
    @Environment(AppState.self) var appState

    var body: some View {
        VStack {
            ToolbarView()

            if let item = appState.navigator.leadHistoryItem {
                PreviewItemView(item: item)
            } else if let pasteStack = appState.history.pasteStack,
                      appState.navigator.pasteStackSelected
            {
                PasteStackPreviewView(pasteStack: pasteStack)
            }
        }
        .padding(.horizontal)
        .padding(.bottom)
        .padding(.top, Popup.verticalPadding)
    }
}
