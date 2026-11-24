import XCTest
@testable import Shop

final class OrderTests: XCTestCase {
    func testAppliesDiscountThroughPublicSurface() {
        var order = Order.create(id: "ord-1", total: 100.0)
        order.applyDiscount(0.2)
        XCTAssertEqual(order.finalTotal(), 80.0)
    }
}
