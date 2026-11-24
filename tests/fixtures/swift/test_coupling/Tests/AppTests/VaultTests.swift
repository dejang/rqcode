import XCTest
@testable import App

final class VaultTests: XCTestCase {
    func makeVault() -> Vault {
        return Vault.create(secretKey: "test-key")
    }

    func testInspectsPrivateInternals() {
        let vault = makeVault()
        _ = vault.rawSecret()
        _ = vault.secretKey
    }

    @Test("resets via debug helper")
    func resetsViaDebugHelper() {
        let vault = makeVault()
        vault.debugReset()
    }
}
