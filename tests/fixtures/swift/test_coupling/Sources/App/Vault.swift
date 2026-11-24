public final class Vault {
    private var secretKey: String

    public init(secretKey: String) {
        self.secretKey = secretKey
    }

    public static func create(secretKey: String) -> Vault {
        return Vault(secretKey: secretKey)
    }

    private func rawSecret() -> String {
        return self.secretKey
    }

    public func verify(_ candidate: String) -> Bool {
        return rawSecret() == candidate
    }

    public func debugReset() {
        self.secretKey = ""
    }
}
