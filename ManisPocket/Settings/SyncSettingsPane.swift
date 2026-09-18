import Defaults
import Settings
import SwiftUI

struct SyncSettingsPane: View {
    private static let sectionContentWidth = SettingsLayout.contentWidth - 150

    @Default(.syncEnabled) private var syncEnabled
    @Default(.syncDeviceName) private var syncDeviceName
    @Default(.syncDiscoverable) private var syncDiscoverable
    @State private var discoveredPeers: [DiscoveredPeer] = []
    @State private var pairedDevices: [PairedDeviceInfo] = []
    @State private var editingDevice: PairedDeviceInfo?
    @State private var editingNickname = ""
    @State private var editingIcon = "💻"
    @State private var showPairingDialog = false
    @State private var pairingPeerID = ""
    @State private var pairingDisplayName = ""
    @State private var pairingPin = ""
    @State private var manualAddress = ""
    @State private var connectionStatus: String?

    var body: some View {
        Settings.Container(contentWidth: SettingsLayout.contentWidth) {
            Settings.Section(label: { Text("Enable") }) {
                Toggle(isOn: $syncEnabled) {
                    Text("Enable Clipboard Sync")
                }
                .onChange(of: syncEnabled) { _, newValue in
                    if newValue {
                        SyncBridge.shared.start()
                    } else {
                        SyncBridge.shared.stop()
                    }
                }
                Toggle(isOn: $syncDiscoverable) {
                    Text("Allow Discovery")
                }
                Text("When enabled, other devices on the same network can find this device automatically via mDNS.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(width: Self.sectionContentWidth, alignment: .leading)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Settings.Section(label: { Text("This Device") }) {
                TextField("Device Name", text: $syncDeviceName)
                    .frame(width: 200)
            }

            Settings.Section(label: { Text("Paired Devices") }) {
                pairedDevicesContent
            }

            Settings.Section(
                bottomDivider: true,
                label: {
                    HStack(spacing: 6) {
                        Text("Discovered Devices")
                        Button {
                            SyncBridge.shared.refreshDiscovery()
                            discoveredPeers.removeAll()
                        } label: {
                            Image(systemName: "arrow.clockwise")
                        }
                        .buttonStyle(.borderless)
                        .controlSize(.small)
                    }
                }
            ) {
                discoveredDevicesContent
            }
        }
        .sheet(item: $editingDevice) { device in editDeviceSheet(device) }
        .sheet(isPresented: $showPairingDialog) { pairingDialogContent }
        .onAppear { pairedDevices = PairedDeviceInfo.loadFromCore() }
        .onReceive(NotificationCenter.default.publisher(for: .syncPeerDiscovered)) { n in
            handlePeerDiscovered(n)
        }
        .onReceive(NotificationCenter.default.publisher(for: .syncPeerLost)) { n in
            handlePeerLost(n)
        }
        .onReceive(NotificationCenter.default.publisher(for: .syncPairingRequest)) { n in
            handlePairingRequest(n)
        }
        .onReceive(NotificationCenter.default.publisher(for: .syncPairingComplete)) { _ in
            pairedDevices = PairedDeviceInfo.loadFromCore()
        }
        .onReceive(NotificationCenter.default.publisher(for: .syncError)) { n in
            if let msg = n.userInfo?["message"] as? String {
                connectionStatus = msg
                DispatchQueue.main.asyncAfter(deadline: .now() + 5) { connectionStatus = nil }
            }
        }
    }

    @ViewBuilder
    private var discoveredDevicesContent: some View {
        if discoveredPeers.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text("No devices found.")
                    .foregroundStyle(.secondary)
                    .controlSize(.small)
                Text("Make sure both devices are on the same network and \"Local Network\" permission is enabled in System Settings → Privacy & Security.")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } else {
            ForEach(discoveredPeers) { peer in
                HStack {
                    Text(peer.displayName)
                    Spacer()
                    Button("Pair") { SyncBridge.shared.requestPairing(peerID: peer.peerID) }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                }
            }
        }

        Divider()

        HStack {
            TextField("IP:Port", text: $manualAddress)
                .frame(width: 200)
            Button("Connect") {
                SyncBridge.shared.addPeerAddress(manualAddress)
                connectionStatus = "Connecting..."
                DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
                    if connectionStatus == "Connecting..." {
                        connectionStatus = nil
                    }
                }
            }
            .buttonStyle(.bordered)
            .controlSize(.small)
            .disabled(manualAddress.isEmpty)
        }

        if let status = connectionStatus {
            Text(status)
                .font(.caption)
                .foregroundStyle(status.contains("fail") || status.contains("error") ? .red : .secondary)
        }
    }

    @ViewBuilder
    private var pairedDevicesContent: some View {
        if pairedDevices.isEmpty {
            Text("No paired devices")
                .foregroundStyle(.secondary)
                .controlSize(.small)
        } else {
            ForEach(pairedDevices) { device in
                pairedDeviceRow(device)
            }
        }
    }

    private func pairedDeviceRow(_ device: PairedDeviceInfo) -> some View {
        HStack {
            Text(device.icon)
            VStack(alignment: .leading) {
                Text(device.nickname).lineLimit(1)
                HStack(spacing: 4) {
                    Circle()
                        .fill(device.isConnected ? Color.green : Color.gray)
                        .frame(width: 6, height: 6)
                    Text(device.isConnected ? "Connected" : "Offline")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
            if device.isAdmin {
                Button { editingDevice = device; editingNickname = device.nickname; editingIcon = device.icon }
                    label: { Image(systemName: "pencil") }
                    .buttonStyle(.borderless)
                    .controlSize(.small)
                Button("Unpair") {
                    SyncBridge.shared.unpair(peerID: device.peerID)
                    AppState.shared.history.core.removePairedPeer(peerId: device.peerID)
                    pairedDevices = PairedDeviceInfo.loadFromCore()
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
            }
        }
        .frame(width: Self.sectionContentWidth)
    }

    private func editDeviceSheet(_ device: PairedDeviceInfo) -> some View {
        VStack(spacing: 16) {
            Text("Edit Device").font(.headline)
            HStack {
                Text("Icon:")
                TextField("", text: $editingIcon).frame(width: 40).font(.title2)
            }
            HStack {
                Text("Name:")
                TextField("Nickname", text: $editingNickname).frame(width: 200)
            }
            HStack {
                Button("Cancel") { editingDevice = nil }.keyboardShortcut(.cancelAction)
                Button("Save") {
                    AppState.shared.history.core.savePairedPeer(peerId: device.peerID, displayName: editingNickname, isAdmin: false)
                    pairedDevices = PairedDeviceInfo.loadFromCore()
                    editingDevice = nil
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(width: 320)
    }

    private var pairingDialogContent: some View {
        VStack(spacing: 16) {
            Text("Pairing Request").font(.headline)
            Text("Device \"\(pairingDisplayName)\" wants to sync clipboards.")
            Text("Confirm this PIN on both devices:")
            HStack(spacing: 8) {
                ForEach(Array(pairingPin.enumerated()), id: \.offset) { _, char in
                    Text(String(char))
                        .font(.system(.title, design: .monospaced))
                        .frame(width: 32, height: 40)
                        .background(Color.primary.opacity(0.05))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                }
            }
            HStack {
                Button("Cancel") {
                    SyncBridge.shared.rejectPairing(peerID: pairingPeerID)
                    showPairingDialog = false
                }
                .keyboardShortcut(.cancelAction)
                Button("Confirm") {
                    SyncBridge.shared.acceptPairing(peerID: pairingPeerID, pin: pairingPin)
                    showPairingDialog = false
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(width: 340)
    }

    private func handlePeerDiscovered(_ notification: NotificationCenter.Publisher.Output) {
        guard let peerID = notification.userInfo?["peerID"] as? String,
              let name = notification.userInfo?["displayName"] as? String else { return }
        SyncBridge.shared.recordPeerName(peerID, name)
        // If already paired, take the chance to fix its display name: pairing saves
        // the raw peerID as the name, but the friendly name only arrives via discovery.
        if let paired = pairedDevices.first(where: { $0.peerID == peerID }) {
            if !name.isEmpty, name != peerID, paired.nickname != name {
                AppState.shared.history.core.savePairedPeer(peerId: peerID, displayName: name, isAdmin: paired.isAdmin)
                pairedDevices = PairedDeviceInfo.loadFromCore()
            }
            return
        }
        // Deduplicate by display name
        discoveredPeers.removeAll { $0.displayName == name }
        discoveredPeers.append(DiscoveredPeer(peerID: peerID, displayName: name))
    }

    private func handlePeerLost(_ notification: NotificationCenter.Publisher.Output) {
        guard let peerID = notification.userInfo?["peerID"] as? String else { return }
        discoveredPeers.removeAll { $0.peerID == peerID }
    }

    private func handlePairingRequest(_ notification: NotificationCenter.Publisher.Output) {
        guard let peerID = notification.userInfo?["peerID"] as? String,
              let name = notification.userInfo?["displayName"] as? String,
              let pin = notification.userInfo?["pin"] as? String else { return }
        SyncBridge.shared.recordPeerName(peerID, name)
        pairingPeerID = peerID
        pairingDisplayName = name
        pairingPin = pin
        showPairingDialog = true
    }
}

private struct DiscoveredPeer: Identifiable {
    let id = UUID()
    let peerID: String
    let displayName: String
}

#Preview {
    SyncSettingsPane()
        .environment(\.locale, .init(identifier: "en"))
}
