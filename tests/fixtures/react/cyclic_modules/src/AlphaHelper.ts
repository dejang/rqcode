import { renderBetaLabel } from "./BetaCard";

export function formatAlpha(value: string): string {
  return `alpha:${value}`;
}

export function describeBeta(value: string): string {
  return renderBetaLabel(value);
}
