import React from "react";
import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Order } from "../../models/order";
import BillingPage from "../../pages/Billing";
import OrdersPage from "../../pages/Orders";
import SettingsPage from "../../pages/Settings";
import { renderWithProviders } from "../../test-utils";
import { ErrorBoundary } from "../ErrorBoundary";
import { OrderCard } from "../OrderCard";

describe("OrderCard", () => {
  it("renders order label and total cleanly", () => {
    const order = new Order(40, "INV-42");
    order.add(15);
    renderWithProviders(
      <ErrorBoundary fallbackMessage="Failed">
        <OrderCard order={order} />
      </ErrorBoundary>,
    );
    expect(screen.getByText("INV-42: $55")).toBeInTheDocument();
  });

  it("renders lazy page routes directly", () => {
    renderWithProviders(
      <div>
        <OrdersPage />
        <SettingsPage />
        <BillingPage />
      </div>,
    );
    expect(screen.getByText("Recent Orders")).toBeInTheDocument();
    expect(screen.getByText("Workspace Settings")).toBeInTheDocument();
    expect(screen.getByText("Billing Overview")).toBeInTheDocument();
  });
});
