import { vi, test, expect } from "vitest";
import { Checkout } from "./index";

test("returns receipt summary and notifies external mailer", async () => {
  const mailer = { sendReceipt: vi.fn(async () => {}) };
  const checkout = new Checkout(mailer);

  const result = await checkout.complete("a@b.com", 500);

  expect(result.paid).toBe(500);
  expect(mailer.sendReceipt).toHaveBeenCalledWith("a@b.com", 500);
});
