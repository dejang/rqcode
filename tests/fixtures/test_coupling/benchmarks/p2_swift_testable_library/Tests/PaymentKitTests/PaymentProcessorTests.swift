import XCTest
@testable import PaymentKit

final class PaymentProcessorTests: XCTestCase {
    func testMutatesImplicitInternalProperty() {
        let proc = PaymentProcessor(signingKey: "s3cr3t")
        proc.maxRetries = 1
        XCTAssertTrue(proc.authorize(amountCents: 500).contains("r=1"))
    }

    func testCallsInternalHelper() {
        let proc = PaymentProcessor(signingKey: "s3cr3t")
        XCTAssertEqual(proc.buildSigningPayload(amountCents: 500), "amt=500;r=3")
    }
}
