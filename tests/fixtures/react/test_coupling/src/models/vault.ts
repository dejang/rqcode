export class VaultService {
  private secret: number;

  constructor(initial: number) {
    this.secret = initial;
  }

  public deposit(amount: number): void {
    if (amount > 0) {
      this.secret = this.rawSecret() + amount;
    }
  }

  public balance(): number {
    return this.rawSecret();
  }

  private rawSecret(): number {
    return this.secret;
  }
}
