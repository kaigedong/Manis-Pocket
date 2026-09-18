import SwiftUI

struct ActionIconButton: View {
    let icon: String
    let isDark: Bool
    let tooltip: String
    let action: () -> Void

    @State private var isHovered = false

    var body: some View {
        Image(systemName: icon)
            .font(.system(size: 11, weight: .semibold))
            .frame(width: 24, height: 22)
            .contentShape(Rectangle())
            .background(
                RoundedRectangle(cornerRadius: 5)
                    .fill(backgroundColor)
            )
            .foregroundStyle(foregroundColor)
            .onHover { hovering in
                withAnimation(.easeInOut(duration: 0.1)) {
                    isHovered = hovering
                }
            }
            .onTapGesture {
                action()
            }
            .help(tooltip)
    }

    private var backgroundColor: Color {
        if !isHovered {
            return .clear
        }
        // Use opaque colors that stand out against glass background
        if icon == "trash" {
            // Delete button: red tint
            return Color.red.opacity(isDark ? 0.5 : 0.15)
        }
        // Copy button: blue tint
        return Color.blue.opacity(isDark ? 0.5 : 0.15)
    }

    private var foregroundColor: Color {
        if isHovered {
            if icon == "trash" {
                return isDark ? .white : Color.red
            }
            return isDark ? .white : Color.blue
        }
        return isDark ? .white.opacity(0.6) : .secondary
    }
}
