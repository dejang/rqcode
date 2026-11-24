import nodemailer from "nodemailer";
import type { Mailer } from "../ports/mailer";

export class SmtpMailer implements Mailer {
  async sendReceipt(to: string, amountCents: number): Promise<void> {
    const transport = nodemailer.createTransport({ jsonTransport: true });
    await transport.sendMail({ to, text: `Paid ${amountCents}` });
  }
}
