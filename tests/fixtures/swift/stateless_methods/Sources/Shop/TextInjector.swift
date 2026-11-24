public class TextInjector {
    private let delayMs: Int

    public init(delayMs: Int) {
        self.delayMs = delayMs
    }

    public static func makeDefault() -> Self {
        return Self(delayMs: 10)
    }

    public static func normalizeLineEndings(_ text: String) -> String {
        return text.replacingOccurrences(of: "\r\n", with: "\n")
    }

    public class func isTrustedProcess(_ pid: Int) -> Bool {
        return pid > 0
    }

    public func inject(_ text: String) -> Int {
        return text.count + self.delayMs
    }
}
