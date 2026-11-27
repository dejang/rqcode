import React from "react";
import { ChildRow } from "./ChildRow";

function Row({ label }: { label: string }) {
  return <span>{label}</span>;
}

export function OrderList({ items }: { items: string[] }) {
  return (
    <section>
      <Row label="Orders" />
      <ul>
        {items.map((item) => (
          <ChildRow key={item} label={item} />
        ))}
      </ul>
    </section>
  );
}
