final class CartController {
    private var items: [Int] = []

    func addItem(priceCents: Int) {
        items.append(priceCents)
    }

    func checkoutTotal() -> Int {
        return applyRounding(items.reduce(0, +))
    }

    private func applyRounding(_ v: Int) -> Int {
        return v
    }
}
