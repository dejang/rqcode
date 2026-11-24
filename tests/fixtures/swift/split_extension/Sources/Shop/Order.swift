public struct Order {
    private var total: Double

    public init(total: Double) {
        self.total = total
    }

    public mutating func applyDiscount(_ rate: Double) {
        self.total -= self.total * rate
    }

    public func amount() -> Double {
        return self.total
    }
}
