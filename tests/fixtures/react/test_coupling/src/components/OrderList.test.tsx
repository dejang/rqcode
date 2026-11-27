import React from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Row } from "../shared/Row";
import { Badge, ObservedCard } from "./cardsBarrel";
import { OrderList } from "./OrderList";

describe("OrderList and shared Row", () => {
  it("renders shared row without colliding with private Row", () => {
    render(
      <div>
        <OrderList items={["first"]} />
        <Row label="Shared row" />
        <ObservedCard title="Live" />
        <Badge text="New" />
      </div>,
    );
    expect(screen.getByText("Shared row")).toBeInTheDocument();
    expect(screen.getByText("Live")).toBeInTheDocument();
    expect(screen.getByText("New")).toBeInTheDocument();
  });
});
