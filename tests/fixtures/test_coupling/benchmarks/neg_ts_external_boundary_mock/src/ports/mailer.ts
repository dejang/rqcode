export interface Mailer {
  sendReceipt(to: string, amountCents: number): Promise<void>;
}
