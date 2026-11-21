import { formatAlpha } from "./alpha";

export function formatBeta(value: string): string {
    return `beta:${value}`;
}

export function callAlpha(value: string): string {
    return formatAlpha(value);
}
