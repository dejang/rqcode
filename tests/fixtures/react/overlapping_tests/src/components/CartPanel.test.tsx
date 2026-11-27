import React from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CartPanel } from "./CartPanel";

describe("CartPanel", () => {
  it("renders subtotal and total for weekday items", () => {
    const items = [10, 20, 30];
    const discount = 5;
    render(<CartPanel prices={items} coupon={discount} />);
    expect(screen.getByTestId("subtotal")).toHaveTextContent("60");
    expect(screen.getByTestId("total")).toHaveTextContent("55");
  });

  it("renders subtotal and total for weekend items", () => {
    const prices = [15, 25, 40];
    const voucher = 10;
    render(<CartPanel prices={prices} coupon={voucher} />);
    expect(screen.getByTestId("subtotal")).toHaveTextContent("80");
    expect(screen.getByTestId("total")).toHaveTextContent("70");
  });

  it("clamps negative totals to zero when empty", () => {
    const emptyPrices: number[] = [];
    const view = render(<CartPanel prices={emptyPrices} coupon={25} />);
    const totalEl = view.container.querySelector("[data-testid='total']");
    expect(totalEl?.textContent).toBe("0");
  });
});
