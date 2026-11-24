export class SyncEngine {
  protected pendingOps: string[] = [];

  public enqueue(op: string): void {
    this.pendingOps.push(op);
    if (process.env.NODE_ENV === "test") {
      this.flushNow();
    }
  }

  public flush(): void {
    this.flushNow();
  }

  protected flushNow(): void {
    this.pendingOps.length = 0;
  }
}
