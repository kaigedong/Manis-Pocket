import Defaults
import SwiftUI

struct AppFilterBar: View {
    @Environment(AppState.self) private var appState

    private var apps: [(bundleId: String, image: ApplicationImage)] {
        appState.history.sourceApps
    }

    private var pairedDevices: [PairedDeviceInfo] {
        appState.syncDevices
    }

    var body: some View {
        let _ = NSLog("[AppFilterBar] apps=\(apps.count) pairedDevices=\(pairedDevices.count) all=\(appState.history.all.count)")
        if !apps.isEmpty || !pairedDevices.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    ForEach(apps, id: \.bundleId) { app in
                        AppFilterIcon(
                            appImage: app.image,
                            isSelected: appState.history.selectedApp == app.bundleId,
                            isDimmed: appState.history.selectedApp != nil
                                && appState.history.selectedApp != app.bundleId,
                            isUnknown: app.bundleId.isEmpty
                        ) {
                            appState.history.toggleAppSelection(app.bundleId)
                        }
                    }

                    if !apps.isEmpty, !pairedDevices.isEmpty {
                        Rectangle()
                            .fill(Color.primary.opacity(0.15))
                            .frame(width: 1, height: 16)
                    }

                    ForEach(pairedDevices) { device in
                        DeviceFilterIcon(
                            device: device,
                            isExcluded: appState.history.excludedDevices.contains(device.peerID)
                        ) {
                            if appState.history.excludedDevices.contains(device.peerID) {
                                appState.history.excludedDevices.remove(device.peerID)
                            } else {
                                appState.history.excludedDevices.insert(device.peerID)
                            }
                        }
                    }
                }
                .padding(.horizontal, 4)
            }
            .frame(height: 26)
        }
    }
}

struct AppFilterIcon: View {
    let appImage: ApplicationImage
    let isSelected: Bool
    let isDimmed: Bool
    var isUnknown: Bool = false
    let action: () -> Void

    @State private var isHovered = false

    var body: some View {
        // "app.badge.questionmark" is not a valid SF Symbol on all macOS versions
        // (returns nil → the previous force-unwrap trapped at launch for items with
        // an empty source app, e.g. anything synced from Android where application=null).
        // Fall back through known-good symbols, then the app's own image — never trap.
        let unknownImage = NSImage(systemSymbolName: "questionmark.app.dashed", accessibilityDescription: "Unknown")
            ?? NSImage(systemSymbolName: "questionmark", accessibilityDescription: "Unknown")
            ?? appImage.nsImage
        Image(nsImage: isUnknown ? unknownImage : appImage.nsImage)
            .resizable()
            .frame(width: 18, height: 18)
            .contentShape(Rectangle())
            .opacity(isDimmed ? 0.45 : 1.0)
            .overlay(
                Group {
                    if isSelected {
                        Color.accentColor.opacity(0.2)
                    } else if isHovered {
                        Color.primary.opacity(0.08)
                    }
                }
            )
            .clipShape(RoundedRectangle(cornerRadius: 4))
            .overlay(
                RoundedRectangle(cornerRadius: 4)
                    .stroke(isSelected ? Color.accentColor.opacity(0.8) : .clear, lineWidth: 1)
            )
            .onHover { hovering in
                isHovered = hovering
            }
            .onTapGesture {
                action()
            }
            .help(isUnknown ? "Unknown Source" : (appImage.bundleIdentifier ?? ""))
    }
}
