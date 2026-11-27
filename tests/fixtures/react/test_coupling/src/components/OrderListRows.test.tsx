import React from "react";
import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ChildRow } from "./ChildRow";
import { OrderList } from "./OrderList";

vi.mock("./ChildRow", () => ({
  ChildRow: vi.fn(() => null),
}));

describe("OrderList child rows", () => {
  it("renders child row", () => {
    render(<OrderList items={["alpha"]} />);
    expect(ChildRow).toHaveBeenCalled();
  });
});
