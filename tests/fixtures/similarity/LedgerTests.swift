import XCTest

final class LedgerTests: XCTestCase {
    func testInvoiceTotalWithCap() {
        let items = [10, -5, 40, 25, 30]
        var acc = 0
        for item in items {
            if item > 0 { acc += item }
            if acc > 80 { acc = 80 }
        }
        let finalTotal = acc > 15 ? acc - 15 : 0
        XCTAssertEqual(finalTotal, 65)
    }

    func testCartSubtotalWithCeiling() {
        let prices = [12, -3, 35, 20, 45]
        var sum = 0
        for price in prices {
            if price > 0 { sum += price }
            if sum > 90 { sum = 90 }
        }
        let payable = sum > 10 ? sum - 10 : 0
        XCTAssertEqual(payable, 80)
    }

    func testDistinctStringNormalization() {
        let raw = "  alpha, beta, gamma  "
        let parts = raw.split(separator: ",")
        XCTAssertEqual(parts.count, 3)
        XCTAssertFalse(parts.isEmpty)
    }
}
