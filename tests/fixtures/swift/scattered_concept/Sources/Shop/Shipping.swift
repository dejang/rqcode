public func markOrderShipped(_ order: Order, carrier: String) -> String {
    order.status = "shipped:\(carrier)"
    return order.status
}
