import React from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { VaultService } from "../models/vault";
import { DebugVaultPanel } from "./DebugVaultPanel";
import UnusedBanner from "./UnusedBanner";
import { VaultPanel } from "./VaultPanel";

vi.mock("../gateways/apiClient", () => ({
  fetchVaultSummary: vi.fn(async () => "ok"),
}));

describe("VaultPanel", () => {
  it("renders vault panel and inspects private balance helper", () => {
    render(<VaultPanel initialBalance={50} />);
    const vault = new VaultService(50);
    vault.deposit(10);
    const secret = (vault as any).rawSecret();
    expect(secret).toBe(60);
    expect(screen.getByText("Test vault: 65")).toBeInTheDocument();
  });

  it("renders debug panel and unused banner", () => {
    render(
      <div>
        <DebugVaultPanel balance={25} />
        <UnusedBanner notice="maintenance" />
      </div>,
    );
    expect(screen.getByTestId("debug-vault")).toHaveTextContent("25");
    expect(screen.getByRole("status")).toHaveTextContent("maintenance");
  });
});
