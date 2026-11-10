import Defaults
import SwiftUI

struct HistoryItemView: View {
    @Bindable var item: HistoryItemDecorator
    var previous: HistoryItemDecorator?
    var next: HistoryItemDecorator?
    var index: Int

    @State private var isHovered = false
    @State private var showCopiedToast = false
    @State private var showDeleteConfirmation = false
    @FocusState private var editFocused: Bool

    private var visualIndex: Int? {
        if appState.navigator.isMultiSelectInProgress, item.selectionIndex >= 0 {
            return item.selectionIndex
        }
        return nil
    }

    private var selectionAppearance: SelectionAppearance {
        let previousSelected = previous?.isSelected ?? false
        let nextSelected = next?.isSelected ?? false
        switch (previousSelected, nextSelected) {
        case (true, false):
            return .topConnection
        case (false, true):
            return .bottomConnection
        case (true, true):
            return .topBottomConnection
        default:
            return .none
        }
    }

    @Environment(AppState.self) private var appState

    var body: some View {
        if item.isEditing {
            editingView
        } else {
            normalView
        }
    }

    private var normalView: some View {
        ZStack(alignment: .trailing) {
            ListItemView(
                id: item.id,
                selectionId: item.id,
                appIcon: item.applicationImage,
                image: item.thumbnailImage,
                accessoryImage: item.thumbnailImage != nil ? nil : ColorImage.from(item.title),
                attributedTitle: item.attributedTitle,
                shortcuts: item.shortcuts,
                isSelected: item.isSelected,
                selectionIndex: visualIndex,
                selectionAppearance: selectionAppearance
            ) {
                Text(verbatim: item.title)
            }
            .padding(.trailing, isHovered && item.isVisible ? 56 : 0)
            .animation(.easeInOut(duration: 0.15), value: isHovered)

            // Copy & Delete buttons (show on hover)
            if isHovered, item.isVisible {
                HStack(spacing: 2) {
                    if item.isFile, item.isRemote {
                        ActionIconButton(
                            icon: "icloud.and.arrow.down",
                            isDark: item.isSelected,
                            tooltip: "Download file"
                        ) {
                            SyncBridge.shared.requestFile(
                                peerId: item.item.syncSource!,
                                filePath: item.title
                            )
                        }
                    }

                    ActionIconButton(
                        icon: "doc.on.doc",
                        isDark: item.isSelected,
                        tooltip: String(localized: "copy_button_tooltip", table: "Localizable")
                    ) {
                        Clipboard.shared.copy(item.item)
                        showCopiedToast = true
                        Task {
                            try? await Task.sleep(for: .seconds(1.5))
                            showCopiedToast = false
                        }
                    }

                    ActionIconButton(
                        icon: "trash",
                        isDark: item.isSelected,
                        tooltip: String(localized: "delete_button_tooltip", table: "Localizable")
                    ) {
                        showDeleteConfirmation = true
                    }
                }
                .padding(.trailing, 8)
                .transition(.opacity)
            }

            // Copied toast overlay
            if showCopiedToast {
                Text(String(localized: "copied_toast", table: "Localizable"))
                    .font(.caption)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(
                        RoundedRectangle(cornerRadius: 4)
                            .fill(Color.green.opacity(0.85))
                    )
                    .foregroundStyle(.white)
                    .padding(.trailing, 8)
                    .transition(.opacity)
            }
        }
        .onAppear {
            item.ensureThumbnailImage()
        }
        .onHover { hovering in
            isHovered = hovering
        }
        .onTapGesture {
            if NSEvent.modifierFlags.contains(.command), appState.multiSelectionEnabled {
                appState.navigator.addToSelection(item: item)
            } else {
                Task {
                    appState.history.select(item)
                }
            }
        }
        .confirmationDialog(
            String(localized: "delete_alert_message", table: "Localizable"),
            isPresented: $showDeleteConfirmation,
            titleVisibility: .visible
        ) {
            Button(String(localized: "delete_alert_confirm", table: "Localizable"), role: .destructive) {
                appState.history.delete(item)
            }
            Button(String(localized: "delete_alert_cancel", table: "Localizable"), role: .cancel) {}
        } message: {
            Text(String(localized: "delete_alert_comment", table: "Localizable"))
        }
        .contextMenu {
            if !item.item.title.isEmpty {
                Button {
                    appState.history.startEditing(item)
                } label: {
                    Text(String(localized: "edit", table: "Localizable"))
                }
            }

            Button {
                appState.history.togglePin(item)
            } label: {
                Text(item.isPinned
                    ? String(localized: "unpin", table: "Localizable")
                    : String(localized: "pin", table: "Localizable"))
            }

            if item.isFile, item.isRemote, let path = item.fileName {
                Button {
                    SyncBridge.shared.requestFile(
                        peerId: item.item.syncSource!,
                        filePath: item.title
                    )
                } label: {
                    Text("Download \(path)")
                }
            }

            Button {
                showDeleteConfirmation = true
            } label: {
                Text(String(localized: "delete", table: "Localizable"))
            }
        }
    }

    private var editingView: some View {
        HStack(spacing: 0) {
            Spacer().frame(width: 10)
            TextField("", text: $item.editingText, axis: .vertical)
                .textFieldStyle(.plain)
                .lineLimit(1 ... 5)
                .focused($editFocused)
                .onAppear {
                    editFocused = true
                }
                .onSubmit {
                    appState.history.saveEditing(item)
                }
                .onExitCommand {
                    appState.history.cancelEditing(item)
                }
            Spacer().frame(width: 4)
            Button {
                appState.history.saveEditing(item)
            } label: {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.green)
            }
            .buttonStyle(.plain)
            .padding(.trailing, 4)
            Button {
                appState.history.cancelEditing(item)
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .padding(.trailing, 8)
        }
        .frame(minHeight: Popup.itemHeight)
        .id(item.id)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.accentColor.opacity(0.15))
        .clipShape(.rect(cornerRadius: Popup.cornerRadius))
    }
}
