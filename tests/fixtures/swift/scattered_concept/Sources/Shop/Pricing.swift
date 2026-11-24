public func applySeasonalDiscount(_ order: Order, rate: Double) -> Double {
    order.total = order.total * (1.0 - rate)
    return order.total
}
