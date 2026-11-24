import XCTest
import AppCore

final class LoggerTests: XCTestCase {
    func testFormat() {
        XCTAssertEqual(Logger.format("hello"), "hello")
    }
}
