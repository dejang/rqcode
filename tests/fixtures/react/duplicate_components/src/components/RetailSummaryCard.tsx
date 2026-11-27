import React from "react";

interface RetailSummaryProps {
  unitPrices: number[];
  taxRate: number;
  loyaltyTier: number;
  heading: string;
}

export const RetailSummaryCard = React.memo(
  ({ unitPrices, taxRate, loyaltyTier, heading }: RetailSummaryProps) => {
    let subtotal = 0;
    for (const price of unitPrices) {
      if (price > 0) {
        subtotal += price;
      }
    }
    const discount = loyaltyTier >= 3 ? subtotal * 0.15 : subtotal * 0.05;
    const finalTotal = Math.round((subtotal - discount) * (1 + taxRate) * 100) / 100;

    return (
      <section className="summary-card">
        <header>
          <h2>{heading}</h2>
        </header>
        <dl>
          <dt>Subtotal</dt>
          <dd>{subtotal}</dd>
          <dt>Total</dt>
          <dd>{finalTotal}</dd>
        </dl>
        <button type="button" disabled={finalTotal <= 0}>
          Confirm
        </button>
      </section>
    );
  },
);
