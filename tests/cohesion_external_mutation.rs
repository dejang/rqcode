mod support;

use std::error::Error;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

// ---------------------------------------------------------------------------
// Group A: external-field-mutation (§4.1)
// ---------------------------------------------------------------------------

#[test]
fn external_field_mutation_flags_foreign_method_and_trait_impl_in_rust()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let diags = analyze_project(
        &[
            ("src/lib.rs", "pub mod account;\npub mod checkout;\n"),
            (
                "src/account.rs",
                "pub struct Account { pub balance: i64 }\n",
            ),
            (
                "src/checkout.rs",
                r"
use crate::account::Account;

pub trait Handler { fn handle(&self, account: &mut Account); }
pub struct Checkout { fee: i64 }

impl Checkout {
    pub fn charge(&self, account: &mut Account) {
        account.balance -= self.fee;
    }
}

impl Handler for Checkout {
    fn handle(&self, account: &mut Account) {
        account.balance = 0;
    }
}
",
            ),
        ],
        &cfg,
    )?;
    let hits = diags_for(&diags, "external-field-mutation");
    assert_eq!(
        hits.len(),
        2,
        "expected both inherent method and trait impl mutating foreign Account to fire, got: {diags:?}"
    );
    assert!(
        hits.iter()
            .any(|d| d.message().contains("Checkout::charge"))
    );
    assert!(
        hits.iter()
            .any(|d| d.message().contains("Checkout::handle"))
    );
    assert!(
        hits.iter().all(|d| !d.message().contains("free function")),
        "method diagnostics must not label methods as free functions: {hits:?}"
    );

    // Config toggle suppresses method mutations too
    let mut disabled = cohesion_only_config();
    disabled.cohesion.flag_external_field_mutation = false;
    let suppressed = analyze_project(
        &[
            ("src/lib.rs", "pub mod account;\npub mod checkout;\n"),
            (
                "src/account.rs",
                "pub struct Account { pub balance: i64 }\n",
            ),
            (
                "src/checkout.rs",
                "use crate::account::Account;\npub struct Checkout { fee: i64 }\nimpl Checkout { pub fn charge(&self, a: &mut Account) { a.balance -= self.fee; } }\n",
            ),
        ],
        &disabled,
    )?;
    assert_no_rule(&suppressed, "external-field-mutation");
    Ok(())
}

#[test]
fn external_field_mutation_flags_foreign_method_in_ts_and_swift() -> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    let ts_diags = analyze_project(
        &[
            (
                "src/account.ts",
                "export class Account { balance: number = 0; }\n",
            ),
            (
                "src/checkout.ts",
                r"
import { Account } from './account';

export class Checkout {
    private fee: number = 5;
    charge(account: Account): void {
        account.balance -= this.fee;
    }
}
",
            ),
        ],
        &cfg,
    )?;
    let ts_mut = diags_for(&ts_diags, "external-field-mutation");
    assert_eq!(
        ts_mut.len(),
        1,
        "expected TS method mutating foreign field to emit external-field-mutation, got: {ts_diags:?}"
    );
    assert!(ts_mut[0].message().contains("Checkout.charge"));

    // Swift: foreign class method AND separate-file extension method mutating foreign Account
    let swift_diags = analyze_project(
        &[
            (
                "Sources/Shop/Account.swift",
                "public class Account { public var balance: Int = 0 }\n",
            ),
            (
                "Sources/Shop/Checkout.swift",
                r"
public class Checkout {
    private let fee: Int = 5
    public func charge(_ account: Account) { account.balance -= self.fee }
}
",
            ),
            (
                "Sources/Shop/Invoice.swift",
                "public class Invoice { public var id: Int = 0 }\n",
            ),
            (
                "Sources/Shop/InvoiceSettle.swift",
                r"
extension Invoice {
    public func settle(_ account: Account) { account.balance = 0 }
}
",
            ),
        ],
        &cfg,
    )?;
    let swift_mut = diags_for(&swift_diags, "external-field-mutation");
    assert_eq!(
        swift_mut.len(),
        2,
        "expected Swift class method and foreign extension method to emit external-field-mutation, got: {swift_diags:?}"
    );
    assert!(
        swift_mut
            .iter()
            .any(|d| d.message().contains("Checkout.charge"))
    );
    assert!(
        swift_mut
            .iter()
            .any(|d| d.message().contains("Invoice.settle"))
    );
    Ok(())
}

#[test]
fn external_field_mutation_groups_by_func_type_and_respects_module_and_test_scope()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // Multi-field dedup in 1 method (1 diag naming both fields, span on method, related on type)
    // + 1 function mutating 2 types (2 diags)
    // + same-module write (0) + #[cfg(test)] write (0)
    let rust_diags = analyze_project(
        &[
            (
                "src/lib.rs",
                "pub mod account;\npub mod wallet;\npub mod ops;\n",
            ),
            (
                "src/account.rs",
                r"
pub struct Account { pub balance: i64, pub fee: i64 }
pub struct LocalHelper;
impl LocalHelper {
    pub fn reset_local(&self, a: &mut Account) { a.balance = 0; }
}
",
            ),
            ("src/wallet.rs", "pub struct Wallet { pub credits: i64 }\n"),
            (
                "src/ops.rs",
                r"
use crate::account::Account;
use crate::wallet::Wallet;

pub struct Runner;
impl Runner {
    pub fn clear_account(&self, a: &mut Account) {
        a.balance = 0;
        a.fee = 0;
        a.balance = 1;
    }
}

pub fn clear_both(a: &mut Account, w: &mut Wallet) {
    a.balance = 0;
    w.credits = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutates_in_test() {
        let mut a = Account { balance: 1, fee: 1 };
        a.balance = 0;
    }
}
",
            ),
        ],
        &cfg,
    )?;
    let hits = diags_for(&rust_diags, "external-field-mutation");
    assert_eq!(
        hits.len(),
        3,
        "expected 1 grouped diag for Runner::clear_account + 2 diags for clear_both, 0 for same-module/test, got: {rust_diags:?}"
    );
    let grouped = hits
        .iter()
        .find(|d| d.message().contains("Runner::clear_account"))
        .unwrap_or_else(|| panic!("missing grouped diagnostic: {hits:?}"));
    assert!(grouped.message().contains("balance") && grouped.message().contains("fee"));
    assert!(grouped.span().file().ends_with("src/ops.rs"));
    assert!(
        grouped
            .related()
            .iter()
            .any(|r| r.span().file().ends_with("src/account.rs")),
        "related location must point to target type definition: {grouped:?}"
    );

    // TS *.test.ts writes must produce 0 diagnostics
    let ts_test_diags = analyze_project(
        &[
            (
                "src/account.ts",
                "export class Account { balance: number = 0; }\n",
            ),
            (
                "src/account.test.ts",
                "import { Account } from './account';\ntest('mutates', () => { const a = new Account(); a.balance = 5; });\n",
            ),
        ],
        &cfg,
    )?;
    assert_no_rule(&ts_test_diags, "external-field-mutation");
    Ok(())
}

#[test]
fn external_field_mutation_flags_inferred_locals_and_typed_expressions()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // Rust: both associated factory (Account::open) and free-function return-type inference (load_account)
    let rust_diags = analyze_project(
        &[
            ("src/lib.rs", "pub mod account;\npub mod ops;\n"),
            (
                "src/account.rs",
                r"
pub struct Account { pub balance: i64 }
impl Account {
    pub fn open() -> Account { Account { balance: 100 } }
}
pub fn load_account() -> Account { Account { balance: 50 } }
",
            ),
            (
                "src/ops.rs",
                r"
use crate::account::{Account, load_account};

pub fn reset_via_assoc() -> Account {
    let mut acc = Account::open();
    acc.balance = 0;
    acc
}

pub fn reset_via_free() -> Account {
    let mut acc = load_account();
    acc.balance = 0;
    acc
}
",
            ),
        ],
        &cfg,
    )?;
    let rust_mut = diags_for(&rust_diags, "external-field-mutation");
    assert_eq!(
        rust_mut.len(),
        2,
        "expected both associated and free-function factory local mutations in Rust to fire, got: {rust_diags:?}"
    );

    // TypeScript: static factory local AND typed expression `(raw as Account).balance = 0`
    let ts_diags = analyze_project(
        &[
            (
                "src/account.ts",
                r"
export class Account {
    balance: number = 100;
    static open(): Account { return new Account(); }
}
",
            ),
            (
                "src/ops.ts",
                r"
import { Account } from './account';

export function resetDefault(): Account {
    const acc = Account.open();
    acc.balance = 0;
    return acc;
}

export function resetCast(raw: unknown): void {
    (raw as Account).balance = 0;
}
",
            ),
        ],
        &cfg,
    )?;
    let ts_mut = diags_for(&ts_diags, "external-field-mutation");
    assert_eq!(
        ts_mut.len(),
        2,
        "expected TS factory local and cast expression mutations to fire, got: {ts_diags:?}"
    );
    Ok(())
}

#[test]
fn external_field_mutation_ignores_undeclared_fields_and_interfaces() -> Result<(), Box<dyn Error>>
{
    let cfg = cohesion_only_config();

    let rust_diags = analyze_project(
        &[
            ("src/lib.rs", "pub mod wrapper;\npub mod ops;\n"),
            ("src/wrapper.rs", "pub struct Wrapper { pub inner: i64 }\n"),
            (
                "src/ops.rs",
                "use crate::wrapper::Wrapper;\npub fn tweak(w: &mut Wrapper) { w.deref_only_field = 42; }\n",
            ),
        ],
        &cfg,
    )?;
    assert_no_rule(&rust_diags, "external-field-mutation");

    // TS interface is non-stateful -> 0 diagnostics
    let ts_diags = analyze_project(
        &[
            (
                "src/account.ts",
                "export interface AccountShape { balance: number; }\n",
            ),
            (
                "src/ops.ts",
                "import { AccountShape } from './account';\nexport function zero(a: AccountShape) { a.balance = 0; }\n",
            ),
        ],
        &cfg,
    )?;
    assert_no_rule(&ts_diags, "external-field-mutation");
    Ok(())
}

#[test]
fn external_field_mutation_excludes_owner_self_and_owner_non_self_param()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // Swift cross-file extension mutating self -> split-impl only, NOT external-field-mutation
    let swift_diags = analyze_project(
        &[
            (
                "Sources/Shop/Account.swift",
                "public class Account { public var balance: Int = 0 }\n",
            ),
            (
                "Sources/Shop/AccountCredit.swift",
                r"
extension Account {
    public func credit(_ amount: Int) { self.balance += amount }
    public func absorb(_ other: Account) { other.balance = 0 }
}
",
            ),
        ],
        &cfg,
    )?;
    assert!(
        diags_for(&swift_diags, "external-field-mutation").is_empty(),
        "Swift cross-file extension mutating self or another Account param must NOT emit external-field-mutation, got: {swift_diags:?}"
    );
    assert_eq!(diags_for(&swift_diags, "split-impl").len(), 1);

    // Rust cross-module trait impl mutating self AND mutating non-self `other: &mut Account`
    // (specifically exercises `func.resolved_owner != Some(type_id)` independently of `receiver_is_self`)
    let rust_diags = analyze_project(
        &[
            ("src/lib.rs", "pub mod account;\npub mod reset;\n"),
            (
                "src/account.rs",
                "pub struct Account { pub balance: i64 }\n",
            ),
            (
                "src/reset.rs",
                r"
use crate::account::Account;

pub trait Resettable { fn reset(&mut self); }
pub trait Absorb { fn absorb(&mut self, other: &mut Account); }

impl Resettable for Account {
    fn reset(&mut self) { self.balance = 0; }
}

impl Absorb for Account {
    fn absorb(&mut self, other: &mut Account) { other.balance = 0; }
}
",
            ),
        ],
        &cfg,
    )?;
    assert!(
        diags_for(&rust_diags, "external-field-mutation").is_empty(),
        "cross-module trait impl mutating self or non-self owner param must NOT emit external-field-mutation, got: {rust_diags:?}"
    );
    assert_no_rule(&rust_diags, "split-impl");
    Ok(())
}

