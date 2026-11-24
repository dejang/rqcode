@main
struct MyApp {
    static func main() {
        let cart = CartController()
        cart.addItem(priceCents: 1200)
        print(cart.checkoutTotal())
    }
}
