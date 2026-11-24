import { applyVipDiscount } from "./internal/discountPipeline";

export class CheckoutService {
  public checkout(items: number[], tier: string): number {
    this.validateItems(items);
    const subtotal = items.reduce((a, b) => a + b, 0);
    return applyVipDiscount(subtotal, tier);
  }

  public validateItems(items: number[]): void {
    if (items.length === 0) {
      throw new Error("empty");
    }
  }
}
