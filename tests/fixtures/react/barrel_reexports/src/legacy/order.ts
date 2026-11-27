export class Order {
  public reference: string;

  constructor(reference: string) {
    this.reference = reference;
  }

  public code(): string {
    return this.reference;
  }
}
