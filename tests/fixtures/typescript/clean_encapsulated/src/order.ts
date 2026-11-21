export class Order {
    private total: number;

    constructor(initialTotal: number) {
        this.total = initialTotal;
    }

    public add(amount: number): void {
        if (amount > 0) {
            this.total += amount;
        }
    }

    public getTotal(): number {
        return this.total;
    }
}
