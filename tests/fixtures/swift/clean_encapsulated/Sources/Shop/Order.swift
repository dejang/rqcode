public struct Order {
    private let id: String
    private var total: Double

    public init(id: String, total: Double) {
        self.id = id
        self.total = total
    }

    public static func create(id: String, total: Double) -> Order {
        return Order(id: id, total: total)
    }

    private mutating func clampTotal() {
        if self.total < 0 {
            self.total = 0
        }
    }

    public mutating func applyDiscount(_ rate: Double) {
        self.total -= self.total * rate
        clampTotal()
    }

    public func finalTotal() -> Double {
        return self.total
    }
}

extension Order: CustomStringConvertible {
    public var description: String {
        return "\(self.id): \(self.total)"
    }
}
