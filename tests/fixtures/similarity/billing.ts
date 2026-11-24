export function scoreCart(prices: number[], coupon: number, cap: number): number {
  let total = 0;
  for (const price of prices) { if (price > 0) { total += price; } if (total > cap) { total = cap; } }
  if (total > coupon) { return total - coupon; }
  return 0;
}
export function scoreOrder(amounts: number[], rebate: number, limit: number): number {
  let sum = 0; const fallback = 0;
  for (const amount of amounts) { if (amount > 0) { sum += amount; } if (sum > limit) { sum = limit; } }
  if (sum > rebate) { return sum - rebate; }
  return fallback;
}
export function buildUser(a: string, b: string, c: string, d: string, e: string, f: string) {
  return { first: a, second: b, third: c, fourth: d, fifth: e, sixth: f };
}
export function buildConfig(p: string, q: string, r: string, s: string, t: string, u: string) {
  return { host: p, port: q, user: r, pass: s, db: t, pool: u };
}
export function runAuth(raw: string): string {
  const t = parseToken(raw); const c = verifySig(t); return formatAudit(loadSession(c));
}
export function runBilling(raw: string): string {
  const d = decodeInvoice(raw); const l = applyDiscount(d); return emitReceipt(commitLedger(l));
}
