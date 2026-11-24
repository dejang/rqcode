import type { Mailer } from "./ports/mailer";

export class Checkout {
  constructor(private readonly mailer: Mailer) {}

  public async complete(email: string, cents: number): Promise<{ paid: number }> {
    await this.mailer.sendReceipt(email, cents);
    return { paid: cents };
  }
}
