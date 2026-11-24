export class CheckoutService {
  totalCents(items: number[]): number {
    return items.reduce((acc, item) => acc + item, 0);
  }
}
