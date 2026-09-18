import AppKit

enum Accessibility {
    private static var allowed: Bool {
        AXIsProcessTrustedWithOptions(nil)
    }

    static func check() {
        guard !allowed else {
            return
        }
    }
}
