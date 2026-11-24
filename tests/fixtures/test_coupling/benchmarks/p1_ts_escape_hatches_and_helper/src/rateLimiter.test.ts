import { RateLimiter } from "./index";

function readInternalBuckets(rl: any) {
  return rl["buckets"];
}

describe("RateLimiter", () => {
  it("invokes private method via bracket notation", () => {
    const limiter = new RateLimiter();
    limiter.allow("u1", 10);
    limiter["evictStale"](2000);
    if (!limiter.allow("u1", 2001)) {
      throw new Error("expected allow after eviction");
    }
  });

  it("reads private field via double cast", () => {
    const limiter = new RateLimiter();
    limiter.allow("u1", 10);
    const map = (limiter as unknown as { buckets: Map<string, number> }).buckets;
    if (map.size !== 1) {
      throw new Error("unexpected size");
    }
  });

  it("destructures private field after any cast", () => {
    const limiter = new RateLimiter();
    limiter.allow("u1", 10);
    const { buckets } = limiter as any;
    if (buckets.size !== 1) {
      throw new Error("unexpected size");
    }
  });

  it("bypasses privacy with ts-expect-error", () => {
    const limiter = new RateLimiter();
    limiter.allow("u1", 10);
    // @ts-expect-error inspecting private buckets
    if (limiter.buckets.size !== 1) {
      throw new Error("unexpected size");
    }
  });

  it("reads private field through untyped helper", () => {
    const limiter = new RateLimiter();
    limiter.allow("u1", 10);
    const buckets = readInternalBuckets(limiter);
    if (buckets.size !== 1) {
      throw new Error("unexpected size");
    }
  });
});
