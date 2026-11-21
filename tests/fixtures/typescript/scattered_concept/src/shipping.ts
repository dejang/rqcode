import { Order } from "./order";

export function applyShippingTax(order: Order, rate: number): void {
    order.tax = order.total * rate;
}
