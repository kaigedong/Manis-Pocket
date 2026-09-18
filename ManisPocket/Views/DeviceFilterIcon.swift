import SwiftUI

struct DeviceFilterIcon: View {
    let device: PairedDeviceInfo
    let isExcluded: Bool
    let action: () -> Void

    @State private var isHovered = false

    var body: some View {
        Text(device.icon)
            .font(.system(size: 14))
            .frame(width: 18, height: 18)
            .background(
                RoundedRectangle(cornerRadius: 4)
                    .fill(isExcluded ? Color.red.opacity(0.1) :
                        isHovered ? Color.primary.opacity(0.08) : Color.clear)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 4)
                    .stroke(isExcluded ? Color.red.opacity(0.5) : .clear, lineWidth: 1)
            )
            .opacity(isExcluded ? 0.3 : 1.0)
            .onHover { isHovered = $0 }
            .onTapGesture { action() }
            .help(device.nickname)
    }
}
