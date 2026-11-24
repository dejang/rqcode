public enum Logger {
    public static func format(_ message: String) -> String {
        #if DEBUG
        print("[DEBUG] \(message)")
        #endif
        return message
    }
}
