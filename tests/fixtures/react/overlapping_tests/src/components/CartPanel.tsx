import React from "react";

export interface CartPanelProps {
  prices: number[];
  coupon: number;
}

export function CartPanel({ prices, coupon }: CartPanelProps) {
  const subtotal = prices.reduce((sum, item) => sum + item, 0);
  const total = Math.max(0, subtotal - coupon);
  return (
    <section aria-label="Cart">
      <p data-testid="subtotal">{subtotal}</p>
      <p data-testid="total">{total}</p>
    </section>
  );
}
