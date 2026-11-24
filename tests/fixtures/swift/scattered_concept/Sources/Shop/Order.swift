public final class Order {
    public var total: Double
    public var weight: Double
    public var status: String

    public init(total: Double, weight: Double, status: String) {
        self.total = total
        self.weight = weight
        self.status = status
    }
}
