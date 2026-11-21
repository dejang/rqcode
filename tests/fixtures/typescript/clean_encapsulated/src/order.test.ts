import { Order } from "./order";

describe("Order", () => {
    it("accumulates positive amounts through public methods", () => {
        const order = new Order(10);
        order.add(15);
        expect(order.getTotal()).toBe(25);
    });

    it("ignores non-positive amounts across multiple additions", () => {
        const order = new Order(10);
        for (const amount of [0, -5, -10]) {
            order.add(amount);
        }
        if (order.getTotal() !== 10) {
            throw new Error("expected total to remain unchanged");
        }
    });
});
