import React, { useRef } from "react";
import { Order } from "../models/order";
import { ForwardedInput } from "./ForwardedInput";
import { MemoSummary } from "./MemoSummary";
import styles from "./OrderCard.module.css";

export interface OrderCardProps {
  order: Order;
}

export function OrderCard({ order }: OrderCardProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  return (
    <article className={styles.card}>
      <h2>{order.label()}</h2>
      <MemoSummary total={order.getTotal()} />
      <ForwardedInput ref={inputRef} label="Promo code" />
    </article>
  );
}
