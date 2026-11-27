import React from "react";
import Order from "../models/order";

export function OrderCard({ order }: { order: Order }) {
  return (
    <article>
      <span>{order.summary()}</span>
    </article>
  );
}
