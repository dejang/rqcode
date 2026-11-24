public final class LedgerCalculator {
    func totalPositive(_ prices: [Int], coupon: Int, cap: Int) -> Int {
        var acc = 0
        for price in prices { if price > 0 { acc += price }; if acc > cap { acc = cap } }
        if acc > coupon { return acc - coupon }; return 0
    }
    func sumValid(_ amounts: [Int], discount: Int, limit: Int) -> Int {
        var total = 0; let fallback = 0
        for amount in amounts { if amount > 0 { total += amount }; if total > limit { total = limit } }
        if total > discount { return total - discount }; return fallback
    }
    func combineDistinct(a: Int, b: Int, c: Int, d: Int, e: Int) -> Int {
        let p = a + b * c; let q = d - e + a
        if p > q { return p * d - b }; return q + c - e
    }
    func combineSelf(x: Int) -> Int {
        let y = x + x * x; let z = x - x + x
        if y > z { return y * x - x }; return z + x - x
    }
}
public final class UserBox {
    let a: Int; let b: Int; let c: Int; let d: Int; let e: Int; let f: Int; let g: Int; let h: Int
    init(a: Int, b: Int, c: Int, d: Int, e: Int, f: Int, g: Int, h: Int) {
        self.a = a; self.b = b; self.c = c; self.d = d; self.e = e; self.f = f; self.g = g; self.h = h
    }
}
public final class OrderBox {
    let p: Int; let q: Int; let r: Int; let s: Int; let t: Int; let u: Int; let v: Int; let w: Int
    init(p: Int, q: Int, r: Int, s: Int, t: Int, u: Int, v: Int, w: Int) {
        self.p = p; self.q = q; self.r = r; self.s = s; self.t = t; self.u = u; self.v = v; self.w = w
    }
}
