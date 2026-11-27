import React from "react";
import { formatAlpha } from "./AlphaHelper";

export function renderBetaLabel(value: string): string {
  return `beta:${value}`;
}

export function BetaCard({ value }: { value: string }) {
  return <div>{formatAlpha(value)}</div>;
}
