import React from "react";
import { Order } from "../models/order";

export const ShippingPanel: React.FC<{
  backupOrder: Order;
  order?: Order;
  rate?: number;
}> = ({ backupOrder, order: activeOrder = backupOrder, rate = 0.1 }) => {
  const applyTax = () => {
    activeOrder.tax = activeOrder.total * rate;
  };

  return (
    <section>
      <span>{activeOrder.tax}</span>
      <button type="button" onClick={applyTax}>
        Calculate tax
      </button>
    </section>
  );
};
