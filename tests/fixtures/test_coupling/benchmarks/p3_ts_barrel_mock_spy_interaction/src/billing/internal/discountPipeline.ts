export function applyVipDiscount(subtotal: number, tier: string): number {
  return tier === "VIP" ? subtotal * 0.9 : subtotal;
}
