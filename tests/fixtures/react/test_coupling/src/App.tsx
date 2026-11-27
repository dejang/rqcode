import React from "react";
import { Badge, ObservedCard } from "./components/cardsBarrel";
import { OrderList } from "./components/OrderList";
import { VaultPanel } from "./components/VaultPanel";
import { fetchVaultSummary } from "./gateways/apiClient";
import { Row as SharedRow } from "./shared/Row";

export default () => {
  void fetchVaultSummary();

  return (
    <main>
      <VaultPanel initialBalance={100} />
      <OrderList items={["alpha", "beta"]} />
      <SharedRow label="Summary" />
      <ObservedCard title="Live" />
      <Badge text="New" />
    </main>
  );
};
