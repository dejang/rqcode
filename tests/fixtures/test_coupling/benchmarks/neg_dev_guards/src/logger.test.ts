import { describe, expect, it } from "vitest";
import { warnDeprecated } from "./logger";

describe("warnDeprecated", () => {
  it("returns feature name", () => {
    expect(warnDeprecated("legacyAuth")).toBe("legacyAuth");
  });
});
