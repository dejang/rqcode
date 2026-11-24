extension Order {
    public func formattedSummary() -> String {
        return "Order total: \(amount())"
    }
}

extension Order: CustomStringConvertible {
    public var description: String {
        return formattedSummary()
    }
}
