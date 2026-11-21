import { Order } from "./order";

export function applyDiscount(order: Order, discount: number): void {
    order.total = Math.max(0, order.total - discount);
}
