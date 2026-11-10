
class ApplicationImageCache {
    static let shared = ApplicationImageCache()

    private let universalClipboardIdentifier: String =
        "com.apple.finder.Open-iCloudDrive"
    private let fallback = ApplicationImage(bundleIdentifier: nil)
    private var cache: [String: ApplicationImage] = [:]

    func getImage(application: String?) -> ApplicationImage {
        guard let bundleIdentifier = application else {
            return fallback
        }

        if let image = cache[bundleIdentifier] {
            return image
        }

        let image = ApplicationImage(bundleIdentifier: bundleIdentifier)
        cache[bundleIdentifier] = image

        return image
    }
}
