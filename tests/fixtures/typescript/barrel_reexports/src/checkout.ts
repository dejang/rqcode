import { Order } from "./models";

export function applyCheckoutDiscount(order: Order, discount: number): void {
  order.total -= discount;
}
