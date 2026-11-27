export default class Order {
  public total: number;

  constructor(total: number) {
    this.total = total;
  }

  public summary(): number {
    return this.total;
  }
}
