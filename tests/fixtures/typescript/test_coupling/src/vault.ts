export class Vault {
  private secret: number;

  constructor(initial: number) {
    this.secret = initial;
  }

  public deposit(n: number): void {
    if (n > 0) {
      this.secret = this.rawSecret() + n;
    }
  }

  public balance(): number {
    return this.rawSecret();
  }

  private rawSecret(): number {
    return this.secret;
  }
}

export function debugReset(vault: Vault): void {
  vault.deposit(0);
}
