export class RateLimiter {
  private buckets = new Map<string, number>();

  public allow(key: string, now: number): boolean {
    this.evictStale(now);
    const count = this.buckets.get(key) ?? 0;
    if (count >= 3) {
      return false;
    }
    this.buckets.set(key, count + 1);
    return true;
  }

  private evictStale(now: number): void {
    if (now > 1000) {
      this.buckets.clear();
    }
  }
}
