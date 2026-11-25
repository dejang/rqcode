mod support;

use std::error::Error;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

// ---------------------------------------------------------------------------
// Group B: leaky-module-boundary (§4.1)
// ---------------------------------------------------------------------------

#[test]
fn leaky_module_boundary_flags_rust_destructuring_inferred_local_and_struct_init()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    let destruct_diags = analyze_project(
        &[
            ("src/lib.rs", "pub mod order;\npub mod billing;\n"),
            (
                "src/order.rs",
                r"
pub struct Order { pub(crate) cents: u64, pub(crate) tax: u64 }
impl Order {
    pub fn total(&self) -> u64 { self.cents + self.tax }
}
",
            ),
            (
                "src/billing.rs",
                r"
use crate::order::Order;

pub fn sum_order(order: Order) -> u64 {
    let Order { cents, tax } = order;
    cents + tax
}
",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(diags_for(&destruct_diags, "leaky-module-boundary").len(), 1);

    // Free-function inferred local + external struct literal init
    let local_diags = analyze_project(
        &[
            ("src/lib.rs", "pub mod order;\npub mod billing;\n"),
            (
                "src/order.rs",
                r"
pub struct Order { pub(crate) cents: u64 }
impl Order { pub fn cents(&self) -> u64 { self.cents } }
pub fn load_order() -> Order { Order { cents: 100 } }
",
            ),
            (
                "src/billing.rs",
                r"
use crate::order::load_order;

pub fn inspect() -> u64 {
    let order = load_order();
    order.cents
}
",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(diags_for(&local_diags, "leaky-module-boundary").len(), 1);

    let init_diags = analyze_project(
        &[
            ("src/lib.rs", "pub mod order;\npub mod billing;\n"),
            (
                "src/order.rs",
                "pub struct Order { pub(crate) cents: u64 }\nimpl Order { pub fn cents(&self) -> u64 { self.cents } }\n",
            ),
            (
                "src/billing.rs",
                "use crate::order::Order;\npub fn build() -> Order { Order { cents: 10 } }\n",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(diags_for(&init_diags, "leaky-module-boundary").len(), 1);
    Ok(())
}

#[test]
fn leaky_module_boundary_flags_typescript_protected_field_access() -> Result<(), Box<dyn Error>> {
    let ts_diags = analyze_project(
        &[
            (
                "src/order.ts",
                r"
export class Order {
    protected cents: number = 0;
    static create(): Order { return new Order(); }
    total(): number { return this.cents; }
}
",
            ),
            (
                "src/billing.ts",
                r"
import { Order } from './order';

export function inspect(): number {
    const order = Order.create();
    return order.cents;
}
",
            ),
        ],
        &cohesion_only_config(),
    )?;
    assert_eq!(diags_for(&ts_diags, "leaky-module-boundary").len(), 1);
    Ok(())
}

#[test]
fn leaky_module_boundary_excludes_owner_inits_self_and_non_self_owner_params()
-> Result<(), Box<dyn Error>> {
    let diags = analyze_project(
        &[
            ("src/lib.rs", "pub mod order;\npub mod convert;\n"),
            (
                "src/order.rs",
                r"
pub struct Order { pub(crate) cents: u64 }
impl Order { pub fn cents(&self) -> u64 { self.cents } }
pub fn same_module_read(order: &Order) -> u64 { order.cents }
",
            ),
            (
                "src/convert.rs",
                r"
use crate::order::Order;

pub trait FromCents { fn from_cents(cents: u64) -> Self; }
pub trait SameOrder { fn same(&self, other: &Order) -> bool; }

impl FromCents for Order {
    fn from_cents(cents: u64) -> Self { Order { cents } }
}

// Exercises func.resolved_owner == Some(Order) on non-self `other.cents`
impl SameOrder for Order {
    fn same(&self, other: &Order) -> bool { self.cents == other.cents }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_in_test() {
        let o = Order { cents: 1 };
        let _ = o.cents;
    }
}
",
            ),
        ],
        &cohesion_only_config(),
    )?;
    assert_no_rule(&diags, "leaky-module-boundary");
    Ok(())
}

#[test]
fn leaky_module_boundary_exempts_swift_default_internal_but_flags_explicit_internal()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    let implicit_diags = analyze_project(
        &[
            (
                "Sources/Shop/Order.swift",
                "struct Order {\n    var cents: Int\n    func total() -> Int { return cents }\n}\n",
            ),
            (
                "Sources/Shop/Billing.swift",
                "func readCents(_ order: Order) -> Int { return order.cents }\n",
            ),
        ],
        &cfg,
    )?;
    assert_no_rule(&implicit_diags, "leaky-module-boundary");

    let explicit_diags = analyze_project(
        &[
            (
                "Sources/Shop/Order.swift",
                "struct Order {\n    internal var cents: Int\n    func total() -> Int { return cents }\n}\n",
            ),
            (
                "Sources/Shop/Billing.swift",
                "func readCents(_ order: Order) -> Int { return order.cents }\n",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(diags_for(&explicit_diags, "leaky-module-boundary").len(), 1);
    Ok(())
}

