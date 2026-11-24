import { Checkout } from "./checkout";
import { SmtpMailer } from "./gateways/smtpMailer";

export async function run(): Promise<number> {
  const checkout = new Checkout(new SmtpMailer());
  const res = await checkout.complete("ops@example.com", 1000);
  return res.paid;
}
