import Foundation

public final class VaultClient {
    private let signingKey: String

    #if DEBUG
    internal var lastRawPayload: String?
    #endif

    public init(signingKey: String) {
        self.signingKey = signingKey
    }

    public func authorize(amountCents: Int) -> String {
        let payload = "amt=\(amountCents)"
        #if DEBUG
        lastRawPayload = payload
        #endif
        return "\(payload):\(signingKey.count)"
    }
}
