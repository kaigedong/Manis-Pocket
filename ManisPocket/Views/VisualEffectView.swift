import SwiftUI

struct VisualEffectView: NSViewRepresentable {
    let visualEffectView = NSVisualEffectView()

    var material: NSVisualEffectView.Material = .popover
    var blendingMode: NSVisualEffectView.BlendingMode = .behindWindow

    func makeNSView(context _: Context) -> NSVisualEffectView {
        visualEffectView
    }

    func updateNSView(_: NSVisualEffectView, context _: Context) {
        visualEffectView.material = material
        visualEffectView.blendingMode = blendingMode
    }
}

@available(macOS 26.0, *)
struct GlassEffectView: NSViewRepresentable {
    let glassEffectView = NSGlassEffectView()

    var style: NSGlassEffectView.Style = .regular

    func makeNSView(context _: Context) -> NSGlassEffectView {
        glassEffectView
    }

    func updateNSView(_: NSGlassEffectView, context _: Context) {
        glassEffectView.style = style
    }
}

#Preview {
    VisualEffectView(
        material: .popover,
        blendingMode: .behindWindow
    )
}
