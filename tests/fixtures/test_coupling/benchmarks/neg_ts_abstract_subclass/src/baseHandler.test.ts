import { describe, expect, it } from "vitest";
import { BaseHandler } from "./baseHandler";

class UppercaseHandler extends BaseHandler {
  protected handle(payload: string): string {
    return payload.toUpperCase();
  }
}

describe("BaseHandler", () => {
  it("trims before invoking subclass hook", () => {
    const handler = new UppercaseHandler();
    expect(handler.execute("  hello  ")).toBe("HELLO");
  });
});
