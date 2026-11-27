import React from "react";
import { Order } from "../models/order";

export interface PricingProps {
  order: Order;
  discount?: number;
}

export function PricingPanel({ order, discount = 5 }: PricingProps) {
  const applyDiscount = () => {
    order.total = Math.max(0, order.total - discount);
  };

  return (
    <section>
      <span>{order.total}</span>
      <button type="button" onClick={applyDiscount}>
        Apply discount
      </button>
    </section>
  );
}
