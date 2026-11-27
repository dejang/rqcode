import React from "react";

export function DebugVaultPanel({ balance }: { balance: number }) {
  return <pre data-testid="debug-vault">{balance}</pre>;
}
