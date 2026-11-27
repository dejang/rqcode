export function computeDiscountedCost(
  prices: number[],
  rebate: number,
  limit: number,
): number {
  let sum = 0;
  for (const price of prices) {
    if (price > 0) {
      sum += price;
    }
    if (sum > limit) {
      sum = limit;
    }
  }
  return sum > rebate ? sum - rebate : 0;
}
