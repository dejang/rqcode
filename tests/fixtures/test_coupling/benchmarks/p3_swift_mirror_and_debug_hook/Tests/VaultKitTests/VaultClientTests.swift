import XCTest
@testable import VaultKit

final class VaultClientTests: XCTestCase {
    func testReflectsPrivateKeyViaMirror() {
        let client = VaultClient(signingKey: "s3cr3t")
        let key = Mirror(reflecting: client).children.first { $0.label == "signingKey" }?.value as? String
        XCTAssertEqual(key, "s3cr3t")
    }

    func testReadsDebugHook() {
        let client = VaultClient(signingKey: "s3cr3t")
        _ = client.authorize(amountCents: 500)
        XCTAssertEqual(client.lastRawPayload, "amt=500")
    }
}
