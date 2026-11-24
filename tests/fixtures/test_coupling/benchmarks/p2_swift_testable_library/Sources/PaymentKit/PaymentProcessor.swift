import Foundation

public final class PaymentProcessor {
    private let signingKey: String
    var maxRetries: Int = 3

    public init(signingKey: String) {
        self.signingKey = signingKey
    }

    public func authorize(amountCents: Int) -> String {
        let payload = buildSigningPayload(amountCents: amountCents)
        return "\(payload):\(signingKey.count)"
    }

    internal func buildSigningPayload(amountCents: Int) -> String {
        return "amt=\(amountCents);r=\(maxRetries)"
    }
}
