export function warnDeprecated(feature: string): string {
  if (process.env.NODE_ENV !== "production") {
    console.warn(`Deprecated feature: ${feature}`);
  }
  return feature;
}
