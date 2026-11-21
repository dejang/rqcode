mod vault;

fn main() {
    let v = vault::Vault::new(10);
    let _ = v.balance();
}
