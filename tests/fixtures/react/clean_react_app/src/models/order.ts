export class Order {
  private total: number;
  private reference: string;

  constructor(initialTotal: number, reference: string) {
    this.total = initialTotal;
    this.reference = reference;
  }

  public add(amount: number): void {
    if (amount > 0) {
      this.total += amount;
    }
  }

  public getTotal(): number {
    return this.total;
  }

  public label(): string {
    return `${this.reference}: $${this.total}`;
  }
}
