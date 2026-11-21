import { Vault } from "./vault";

export function main(): number {
  const vault = new Vault(100);
  vault.deposit(50);
  return vault.balance();
}
