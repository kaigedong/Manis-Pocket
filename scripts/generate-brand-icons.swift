import AppKit
import Foundation

let iconSet = URL(fileURLWithPath: "ManisPocket/Assets.xcassets/AppIcon.appiconset", isDirectory: true)
let files: [(String, Int)] = [
    ("AppIcon (Big Sur)-16w.png", 16),
    ("AppIcon (Big Sur)-32w.png", 32),
    ("AppIcon (Big Sur)-32w-1.png", 32),
    ("AppIcon (Big Sur)-64w.png", 64),
    ("AppIcon (Big Sur)-128w.png", 128),
    ("AppIcon (Big Sur)-256w.png", 256),
    ("AppIcon (Big Sur)-256w-1.png", 256),
    ("AppIcon (Big Sur)-512w.png", 512),
    ("AppIcon (Big Sur)-512w-1.png", 512),
    ("AppIcon (Big Sur)-1024w.png", 1024),
]

for (name, size) in files {
    guard let image = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: size,
        pixelsHigh: size,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    ), let context = NSGraphicsContext(bitmapImageRep: image) else {
        fatalError("Unable to render icon at \(size) px")
    }

    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = context
    let scale = CGFloat(size) / 1024
    context.cgContext.scaleBy(x: scale, y: scale)

    NSColor(calibratedRed: 0.15, green: 0.20, blue: 0.30, alpha: 1).setFill()
    NSBezierPath(roundedRect: NSRect(x: 44, y: 44, width: 936, height: 936), xRadius: 222, yRadius: 222).fill()

    NSColor(calibratedRed: 0.41, green: 0.86, blue: 0.76, alpha: 1).setFill()
    NSBezierPath(roundedRect: NSRect(x: 406, y: 684, width: 212, height: 66), xRadius: 30, yRadius: 30).fill()

    let pocket = NSBezierPath()
    pocket.lineWidth = 68
    pocket.lineCapStyle = .round
    pocket.lineJoinStyle = .round
    pocket.move(to: NSPoint(x: 284, y: 642))
    pocket.line(to: NSPoint(x: 284, y: 332))
    pocket.curve(to: NSPoint(x: 388, y: 228), controlPoint1: NSPoint(x: 284, y: 270), controlPoint2: NSPoint(x: 326, y: 228))
    pocket.line(to: NSPoint(x: 636, y: 228))
    pocket.curve(to: NSPoint(x: 740, y: 332), controlPoint1: NSPoint(x: 698, y: 228), controlPoint2: NSPoint(x: 740, y: 270))
    pocket.line(to: NSPoint(x: 740, y: 642))
    NSColor.white.setStroke()
    pocket.stroke()

    NSGraphicsContext.restoreGraphicsState()
    guard let data = image.representation(using: .png, properties: [:]) else {
        fatalError("Unable to encode icon at \(size) px")
    }
    try data.write(to: iconSet.appendingPathComponent(name))
}
