import { vi, describe, it, expect } from "vitest";
import { applyVipDiscount } from "../billing/internal/discountPipeline";
import { CheckoutService } from "../billing";

vi.mock("../billing/internal/discountPipeline", () => ({
  applyVipDiscount: vi.fn(() => 90),
}));

describe("CheckoutService", () => {
  it("orchestrates discount pipeline", () => {
    const svc = new CheckoutService();
    const validateSpy = vi.spyOn(svc, "validateItems");

    svc.checkout([50, 50], "VIP");

    expect(validateSpy).toHaveBeenCalledTimes(1);
    expect(applyVipDiscount).toHaveBeenCalledWith(100, "VIP");
  });
});
