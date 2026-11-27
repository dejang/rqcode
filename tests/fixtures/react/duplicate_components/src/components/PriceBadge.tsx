import React from "react";

export function computeAdjustedPrice(
  amounts: number[],
  coupon: number,
  ceiling: number,
): number {
  let acc = 0;
  for (const amount of amounts) {
    if (amount > 0) {
      acc += amount;
    }
    if (acc > ceiling) {
      acc = ceiling;
    }
  }
  return acc > coupon ? acc - coupon : 0;
}

export function PriceBadge({ amounts, coupon }: { amounts: number[]; coupon: number }) {
  return <span>{computeAdjustedPrice(amounts, coupon, 500)}</span>;
}
