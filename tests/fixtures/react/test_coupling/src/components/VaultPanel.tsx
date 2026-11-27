import React from "react";
import { VaultService } from "../models/vault";

export function VaultPanel({ initialBalance }: { initialBalance: number }) {
  const vault = new VaultService(initialBalance);
  vault.deposit(15);
  const label =
    process.env.NODE_ENV === "test"
      ? `Test vault: ${vault.balance()}`
      : `Vault: ${vault.balance()}`;

  return <section>{label}</section>;
}
