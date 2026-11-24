import XCTest
@testable import MyApp

final class CartControllerTests: XCTestCase {
    func testCheckoutTotalViaPublicEntryPoints() {
        let cart = CartController()
        cart.addItem(priceCents: 1200)
        XCTAssertEqual(cart.checkoutTotal(), 1200)
    }
}
