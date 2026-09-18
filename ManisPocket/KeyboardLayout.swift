import Carbon
import Sauce

class KeyboardLayout {
    static var current: KeyboardLayout {
        KeyboardLayout()
    }

    /// Some Dvorak and bépo layouts switch to QWERTY/Azerty while Command is held.
    var commandSwitchesToQWERTY: Bool {
        localizedName.hasSuffix("⌘")
    }

    var localizedName: String {
        if let value = TISGetInputSourceProperty(inputSource, kTISPropertyLocalizedName) {
            Unmanaged<CFString>.fromOpaque(value).takeUnretainedValue() as String
        } else {
            ""
        }
    }

    private var inputSource: TISInputSource!

    init() {
        inputSource = TISCopyCurrentKeyboardLayoutInputSource().takeUnretainedValue()
    }
}
