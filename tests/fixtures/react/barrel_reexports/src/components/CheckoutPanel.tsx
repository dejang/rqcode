import React from "react";
import { Order, OrderCard } from "./index";

export function CheckoutPanel({ order, delta }: { order: Order; delta: number }) {
  const applyAdjustment = () => {
    order.total = order.summary() + delta;
  };

  return (
    <section>
      <OrderCard order={order} />
      <button type="button" onClick={applyAdjustment}>
        Update total
      </button>
    </section>
  );
}
