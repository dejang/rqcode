import { describe, expect, it } from "vitest";
import { CheckoutService } from "./checkoutService";

describe("CheckoutService", () => {
  it("sums line items", () => {
    const service = new CheckoutService();
    expect(service.totalCents([100, 250])).toBe(350);
  });
});
