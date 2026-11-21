export function calculateRetailTotal(
  unitPrices: number[],
  taxRate: number,
  loyaltyTier: number,
): number {
  let subtotal = 0;
  for (const price of unitPrices) {
    if (price > 0) {
      subtotal += price;
    }
  }
  let discount = 0;
  if (loyaltyTier >= 3) {
    discount = subtotal * 0.15;
  } else if (loyaltyTier >= 1) {
    discount = subtotal * 0.05;
  }
  const taxable = Math.max(0, subtotal - discount);
  const taxAmount = taxable * taxRate;
  return Math.round((taxable + taxAmount) * 100) / 100;
}

export function computeWholesaleTotal(
  itemCosts: number[],
  dutyRate: number,
  volumeBand: number,
): number {
  let baseAmount = 0;
  for (const cost of itemCosts) {
    if (cost > 0) {
      baseAmount += cost;
    }
  }
  let rebate = 0;
  if (volumeBand >= 5) {
    rebate = baseAmount * 0.2;
  } else if (volumeBand >= 2) {
    rebate = baseAmount * 0.1;
  }
  const netValue = Math.max(0, baseAmount - rebate);
  const dutyAmount = netValue * dutyRate;
  return Math.round((netValue + dutyAmount) * 100) / 100;
}
