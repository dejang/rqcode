import { Vault, debugReset } from "./vault";

describe("Vault", () => {
  it("deposits funds and inspects raw secret", () => {
    const vault = new Vault(10);
    vault.deposit(25);
    const raw = (vault as any).rawSecret();
    if (raw !== 35) {
      throw new Error("unexpected secret");
    }
  });

  it("resets via debug helper", () => {
    const vault = new Vault(10);
    debugReset(vault);
    if (vault.balance() !== 10) {
      throw new Error("unexpected balance");
    }
  });
});
