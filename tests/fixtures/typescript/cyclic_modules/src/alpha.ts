import { formatBeta } from "./beta";

export function formatAlpha(value: string): string {
    return `alpha:${value}`;
}

export function callBeta(value: string): string {
    return formatBeta(value);
}
