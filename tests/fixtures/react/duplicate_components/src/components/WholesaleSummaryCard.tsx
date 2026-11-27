import React from "react";

interface WholesaleSummaryProps {
  itemCosts: number[];
  dutyRate: number;
  volumeBand: number;
  title: string;
}

export function WholesaleSummaryCard({
  itemCosts,
  dutyRate,
  volumeBand,
  title,
}: WholesaleSummaryProps) {
  let baseAmount = 0;
  for (const cost of itemCosts) {
    if (cost > 0) {
      baseAmount += cost;
    }
  }
  const rebate = volumeBand >= 5 ? baseAmount * 0.2 : baseAmount * 0.1;
  const netPayable = Math.round((baseAmount - rebate) * (1 + dutyRate) * 100) / 100;

  return (
    <section className="summary-card">
      <header>
        <h2>{title}</h2>
      </header>
      <dl>
        <dt>Base</dt>
        <dd>{baseAmount}</dd>
        <dt>Payable</dt>
        <dd>{netPayable}</dd>
      </dl>
      <button type="button" disabled={netPayable <= 0}>
        Submit
      </button>
    </section>
  );
}
