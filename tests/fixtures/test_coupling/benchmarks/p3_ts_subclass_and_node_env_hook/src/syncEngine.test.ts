import { SyncEngine } from "./index";

class CapturingSyncEngine extends SyncEngine {
  public flushCount = 0;

  public getQueue(): string[] {
    return this.pendingOps;
  }

  protected override flushNow(): void {
    this.flushCount += 1;
  }
}

test("queues and flushes via subclass", () => {
  const engine = new CapturingSyncEngine();
  engine.enqueue("create");
  if (engine.getQueue().length !== 1 || engine.flushCount !== 1) {
    throw new Error("unexpected state");
  }
});
