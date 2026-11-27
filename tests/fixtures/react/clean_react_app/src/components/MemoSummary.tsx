import React from "react";

interface MemoSummaryProps {
  total: number;
}

export const MemoSummary = React.memo(({ total }: MemoSummaryProps) => {
  return (
    <aside aria-label="Order summary">
      <strong>Total: {total}</strong>
    </aside>
  );
});
