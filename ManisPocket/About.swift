import Cocoa

class About {
    private var links: NSMutableAttributedString {
        let string = NSMutableAttributedString(string: "GitHub",
                                               attributes: [NSAttributedString.Key.foregroundColor: NSColor.labelColor])
        string.addAttribute(.link, value: "https://github.com/kaigedong/Maccy-plus", range: NSRange(location: 0, length: 6))
        return string
    }

    private var credits: NSMutableAttributedString {
        let credits = NSMutableAttributedString(string: "",
                                                attributes: [NSAttributedString.Key.foregroundColor: NSColor.labelColor])
        credits.append(links)
        credits.append(NSAttributedString(string: "\n\n"))
        credits.append(NSAttributedString(string: "Original work © Alex Rodionov"))
        credits.setAlignment(.center, range: NSRange(location: 0, length: credits.length))
        return credits
    }

    @objc
    func openAbout(_: NSMenuItem?) {
        NSApp.activate(ignoringOtherApps: true)
        NSApp.orderFrontStandardAboutPanel(options: [NSApplication.AboutPanelOptionKey.credits: credits])
    }
}
