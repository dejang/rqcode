mod support;

use rqcode::config::{Config, PublicApiScope};
use std::error::Error;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

// ---------------------------------------------------------------------------
// Group C: scattered-concept & Pure DTO Exemption (§4.1 & §4.2)
// ---------------------------------------------------------------------------

#[test]
fn scattered_concept_detects_inferred_local_field_accesses() -> Result<(), Box<dyn Error>> {
    let diags = analyze_project(
        &[
            (
                "src/lib.rs",
                "pub mod order;\npub mod pricing;\npub mod shipping;\n",
            ),
            (
                "src/order.rs",
                "pub struct Order { pub cents: u64 }\npub fn load_order() -> Order { Order { cents: 100 } }\n",
            ),
            (
                "src/pricing.rs",
                "use crate::order::load_order;\npub fn with_fee() -> u64 { let o = load_order(); o.cents + 5 }\n",
            ),
            (
                "src/shipping.rs",
                "use crate::order::load_order;\npub fn with_ship() -> u64 { let o = load_order(); o.cents + 15 }\n",
            ),
        ],
        &cohesion_only_config(),
    )?;
    assert_eq!(diags_for(&diags, "scattered-concept").len(), 1);
    Ok(())
}

#[test]
fn scattered_concept_coordinator_guard_covers_locals_and_mixed_param_plus_local()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // Pure-local coordinator AND mixed (param + inferred local) coordinator -> 0 scattered-concept
    let read_only = analyze_project(
        &[
            (
                "src/lib.rs",
                "pub mod models;\npub mod pricing;\npub mod shipping;\n",
            ),
            (
                "src/models.rs",
                r"
pub struct Customer { pub tier: u8 }
pub struct Cart { pub subtotal: u64 }
pub fn load_customer() -> Customer { Customer { tier: 2 } }
pub fn load_cart() -> Cart { Cart { subtotal: 50 } }
",
            ),
            (
                "src/pricing.rs",
                r"
use crate::models::{load_cart, load_customer};

pub fn quote_locals() -> u64 {
    let customer = load_customer();
    let cart = load_cart();
    if customer.tier > 1 { cart.subtotal } else { cart.subtotal + 5 }
}
",
            ),
            (
                "src/shipping.rs",
                r"
use crate::models::{Customer, load_cart};

pub fn quote_mixed(customer: &Customer) -> u64 {
    let cart = load_cart();
    if customer.tier > 2 { cart.subtotal + 10 } else { cart.subtotal + 20 }
}
",
            ),
        ],
        &cfg,
    )?;
    assert!(
        diags_for(&read_only, "scattered-concept").is_empty(),
        "read-only coordinators using locals or param+local must NOT fire scattered-concept, got: {read_only:?}"
    );

    // Mutating coordinator STILL fires scattered-concept on the mutated type
    let mutating = analyze_project(
        &[
            (
                "src/lib.rs",
                "pub mod models;\npub mod pricing;\npub mod shipping;\n",
            ),
            (
                "src/models.rs",
                "pub struct Customer { pub tier: u8 }\npub struct Cart { pub subtotal: u64 }\n",
            ),
            (
                "src/pricing.rs",
                "use crate::models::{Customer, Cart};\npub fn apply_tier(c: &Customer, cart: &mut Cart) { if c.tier > 1 { cart.subtotal -= 5; } }\n",
            ),
            (
                "src/shipping.rs",
                "use crate::models::{Customer, Cart};\npub fn add_ship(c: &Customer, cart: &mut Cart) { if c.tier == 0 { cart.subtotal += 10; } }\n",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(diags_for(&mutating, "scattered-concept").len(), 1);
    Ok(())
}

#[test]
fn scattered_concept_anemic_trigger_uses_behavioral_methods_and_comparison_in_rust()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // Positive: self-returning builder (`with_cents(mut self) -> Self`) + `#[cfg(test)]` method
    // + static factory -> 0 inherent behavioral methods! 2 externals (1 mutating) in 1 module fires.
    let positive = analyze_project(
        &[
            ("src/lib.rs", "pub mod order;\npub mod ops;\n"),
            (
                "src/order.rs",
                r"
pub struct Order { pub cents: u64 }
impl Order {
    pub fn zero() -> Self { Self { cents: 0 } }
    pub fn with_cents(mut self, cents: u64) -> Self { self.cents = cents; self }
    #[cfg(test)]
    pub fn probe(&self) -> u64 { self.cents }
}
",
            ),
            (
                "src/ops.rs",
                r"
use crate::order::Order;

pub fn add_fee(order: &mut Order) { order.cents += 5; }
pub fn read_cents(order: &Order) -> u64 { order.cents }
",
            ),
        ],
        &cfg,
    )?;
    let hits = diags_for(&positive, "scattered-concept");
    assert_eq!(
        hits.len(),
        1,
        "type with only factory, self-returning builder, and test method has 0 behavioral methods -> must fire, got: {positive:?}"
    );
    assert!(
        hits[0].message().contains("has 0 "),
        "diagnostic message must report 0 behavioral methods, got: {}",
        hits[0].message()
    );

    // Negative: 2 behavioral methods and 2 externals (1 mutating) in 1 module -> 2 > 2 is false -> 0
    let negative = analyze_project(
        &[
            ("src/lib.rs", "pub mod order;\npub mod ops;\n"),
            (
                "src/order.rs",
                r"
pub struct Order { pub cents: u64 }
impl Order {
    pub fn total(&self) -> u64 { self.cents }
    pub fn tax(&self) -> u64 { self.cents / 10 }
}
",
            ),
            (
                "src/ops.rs",
                r"
use crate::order::Order;

pub fn add_fee(order: &mut Order) { order.cents += 5; }
pub fn read_cents(order: &Order) -> u64 { order.cents }
",
            ),
        ],
        &cfg,
    )?;
    assert!(
        diags_for(&negative, "scattered-concept").is_empty(),
        "2 behavioral methods vs 2 externals in 1 module must NOT fire anemic trigger (requires >), got: {negative:?}"
    );
    Ok(())
}

#[test]
fn scattered_concept_anemic_trigger_excludes_ts_and_swift_lifecycle_methods()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    let ts_diags = analyze_project(
        &[
            (
                "src/order.ts",
                r"
export class Order {
    cents: number;
    constructor(cents: number) { this.cents = cents; }
    static zero(): Order { return new Order(0); }
}
",
            ),
            (
                "src/ops.ts",
                "import { Order } from './order';\nexport function addFee(o: Order): void { o.cents += 5; }\nexport function readCents(o: Order): number { return o.cents; }\n",
            ),
        ],
        &cfg,
    )?;
    let ts_scattered = diags_for(&ts_diags, "scattered-concept");
    assert_eq!(ts_scattered.len(), 1);
    assert!(ts_scattered[0].message().contains("has 0 "));

    let swift_diags = analyze_project(
        &[
            (
                "Sources/Shop/Order.swift",
                r"
public class Order {
    public var cents: Int
    public init(cents: Int) { self.cents = cents }
    deinit {}
}
",
            ),
            (
                "Sources/Shop/Ops.swift",
                "public func addFee(_ o: Order) { o.cents += 5 }\npublic func readCents(_ o: Order) -> Int { return o.cents }\n",
            ),
        ],
        &cfg,
    )?;
    let swift_scattered = diags_for(&swift_diags, "scattered-concept");
    assert_eq!(swift_scattered.len(), 1);
    assert!(swift_scattered[0].message().contains("has 0 "));
    Ok(())
}

#[test]
fn scattered_concept_exempts_pure_read_only_dtos_in_rust_and_swift() -> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    let rust_diags = analyze_project(
        &[
            (
                "src/lib.rs",
                "pub mod dto;\npub mod pricing;\npub mod shipping;\n",
            ),
            (
                "src/dto.rs",
                r"
#[derive(Serialize, Deserialize)]
pub struct OrderDto { pub id: u64, pub cents: u64 }
impl OrderDto { pub fn new(id: u64, cents: u64) -> Self { Self { id, cents } } }
",
            ),
            (
                "src/pricing.rs",
                "use crate::dto::OrderDto;\npub fn subtotal(dto: &OrderDto) -> u64 { dto.cents + dto.id }\n",
            ),
            (
                "src/shipping.rs",
                "use crate::dto::OrderDto;\npub fn weight_fee(dto: &OrderDto) -> u64 { dto.cents / 10 }\n",
            ),
        ],
        &cfg,
    )?;
    assert_no_rule(&rust_diags, "scattered-concept");

    let swift_diags = analyze_project(
        &[
            (
                "Sources/Shop/OrderDto.swift",
                r"
public struct OrderDto: Codable {
    public let id: Int
    public let cents: Int
    public init(id: Int, cents: Int) { self.id = id; self.cents = cents }
}
",
            ),
            (
                "Sources/Shop/Pricing.swift",
                "public func subtotal(_ dto: OrderDto) -> Int { return dto.cents + dto.id }\n",
            ),
            (
                "Sources/Shop/Shipping.swift",
                "public func weightFee(_ dto: OrderDto) -> Int { return dto.cents / 10 }\n",
            ),
        ],
        &cfg,
    )?;
    assert_no_rule(&swift_diags, "scattered-concept");
    Ok(())
}

#[test]
fn scattered_concept_exempts_pure_readonly_typescript_classes() -> Result<(), Box<dyn Error>> {
    let ts_diags = analyze_project(
        &[
            (
                "src/dto.ts",
                r"
export class OrderDto {
    private static version: number = 1;
    readonly id: number;
    readonly cents: number;

    constructor(id: number, cents: number) {
        this.id = id;
        this.cents = cents;
    }
}

export class ParamDto {
    constructor(
        public readonly id: number,
        public readonly cents: number,
    ) {}
}
",
            ),
            (
                "src/pricing.ts",
                r"
import { OrderDto, ParamDto } from './dto';

export function subtotal(dto: OrderDto): number { return dto.cents + dto.id; }
export function paramSubtotal(dto: ParamDto): number { return dto.cents + dto.id; }
",
            ),
            (
                "src/shipping.ts",
                r"
import { OrderDto, ParamDto } from './dto';

export function weightFee(dto: OrderDto): number { return dto.cents / 10; }
export function paramWeightFee(dto: ParamDto): number { return dto.cents / 10; }
",
            ),
        ],
        &cohesion_only_config(),
    )?;
    assert!(
        diags_for(&ts_diags, "scattered-concept").is_empty(),
        "pure read-only TS all-readonly classes (fields or param props, ignoring static) must be exempt, got: {ts_diags:?}"
    );
    Ok(())
}

#[test]
fn scattered_concept_rejects_non_exempt_dtos() -> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let cases: &[(&str, &[(&str, &str)])] = &[
        (
            "Condition 2: derived struct with private field",
            &[
                ("src/lib.rs", "pub mod dto;\npub mod pricing;\npub mod shipping;\n"),
                ("src/dto.rs", "#[derive(Serialize)]\npub struct PartialDto { pub cents: u64, secret: u64 }\n"),
                ("src/pricing.rs", "use crate::dto::PartialDto;\npub fn subtotal(dto: &PartialDto) -> u64 { dto.cents + 1 }\n"),
                ("src/shipping.rs", "use crate::dto::PartialDto;\npub fn fee(dto: &PartialDto) -> u64 { dto.cents / 10 }\n"),
            ],
        ),
        (
            "Condition 3: derived type with behavioral method",
            &[
                ("src/lib.rs", "pub mod dto;\npub mod pricing;\npub mod shipping;\n"),
                ("src/dto.rs", "#[derive(Serialize)]\npub struct BehavioralOrder { pub cents: u64 }\nimpl BehavioralOrder { pub fn doubled(&self) -> u64 { self.cents * 2 } }\n"),
                ("src/pricing.rs", "use crate::dto::BehavioralOrder;\npub fn subtotal(dto: &BehavioralOrder) -> u64 { dto.cents + 1 }\n"),
                ("src/shipping.rs", "use crate::dto::BehavioralOrder;\npub fn fee(dto: &BehavioralOrder) -> u64 { dto.cents / 10 }\n"),
            ],
        ),
        (
            "Condition 4a: takes &mut Dto without direct field write",
            &[
                ("src/lib.rs", "pub mod dto;\npub mod pricing;\npub mod shipping;\n"),
                ("src/dto.rs", "#[derive(Serialize)]\npub struct MutRefDto { pub cents: u64 }\n"),
                ("src/pricing.rs", "use crate::dto::MutRefDto;\nfn touch(_: &mut MutRefDto) {}\npub fn refresh(dto: &mut MutRefDto) { touch(dto); }\n"),
                ("src/shipping.rs", "use crate::dto::MutRefDto;\npub fn fee(dto: &MutRefDto) -> u64 { dto.cents / 10 }\n"),
            ],
        ),
        (
            "Condition 4b: inferred-local mutation revokes DTO exemption",
            &[
                ("src/lib.rs", "pub mod dto;\npub mod pricing;\npub mod shipping;\n"),
                ("src/dto.rs", "#[derive(Serialize)]\npub struct LocalMutDto { pub cents: u64 }\npub fn load_dto() -> LocalMutDto { LocalMutDto { cents: 10 } }\n"),
                ("src/pricing.rs", "use crate::dto::load_dto;\npub fn bump() -> u64 { let mut dto = load_dto(); dto.cents += 5; dto.cents }\n"),
                ("src/shipping.rs", "use crate::dto::LocalMutDto;\npub fn fee(dto: &LocalMutDto) -> u64 { dto.cents / 10 }\n"),
            ],
        ),
    ];
    for (label, files) in cases {
        let diags = analyze_project(files, &cfg)?;
        assert_eq!(
            diags_for(&diags, "scattered-concept").len(),
            1,
            "{label}: expected 1 scattered-concept diagnostic, got: {diags:?}"
        );
    }
    Ok(())
}

#[test]
fn typescript_readonly_dto_rejects_mutable_fields_and_preserves_test_only_field_rule()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // Mutable body field AND mutable constructor parameter property must NOT be exempt
    let mutable_ts = analyze_project(
        &[
            (
                "src/dto.ts",
                r"
export class MutableRecord { readonly id: number = 0; cents: number = 0; }
export class MutableParamDto {
    constructor(public readonly id: number, public cents: number) {}
}
",
            ),
            (
                "src/pricing.ts",
                r"
import { MutableRecord, MutableParamDto } from './dto';
export function s1(d: MutableRecord): number { return d.cents + d.id; }
export function s2(d: MutableParamDto): number { return d.cents + d.id; }
",
            ),
            (
                "src/shipping.ts",
                r"
import { MutableRecord, MutableParamDto } from './dto';
export function f1(d: MutableRecord): number { return d.cents / 10; }
export function f2(d: MutableParamDto): number { return d.cents / 10; }
",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(
        diags_for(&mutable_ts, "scattered-concept").len(),
        2,
        "both mutable-field and mutable-param-property TS classes must NOT be exempt, got: {mutable_ts:?}"
    );

    // Regression guard: is_immutable_record must NOT overload has_implicit_field_readers
    let tc_diags = analyze_project(
        &[
            (
                "src/model.ts",
                r"
export class ReadOnlyModel {
    readonly usedInProd: number;
    readonly testOnlyField: number;

    constructor(usedInProd: number, testOnlyField: number) {
        this.usedInProd = usedInProd;
        this.testOnlyField = testOnlyField;
    }

    total(): number { return this.usedInProd; }
}
",
            ),
            (
                "src/model.test.ts",
                r"
import { ReadOnlyModel } from './model';

test('checks test-only readonly field', () => {
    const m = new ReadOnlyModel(10, 99);
    expect(m.testOnlyField).toBe(99);
});
",
            ),
        ],
        &{
            let mut tc_cfg = Config::default();
            tc_cfg.cohesion.enabled = false;
            tc_cfg.similarity.enabled = false;
            tc_cfg.test_coupling.enabled = true;
            tc_cfg.test_coupling.public_api_scope = PublicApiScope::All;
            tc_cfg.ast_grep.enabled = false;
            tc_cfg.ast_grep.builtin_rules = false;
            tc_cfg
        },
    )?;
    let test_only = diags_for(&tc_diags, "test-only-production-field");
    assert_eq!(test_only.len(), 1);
    assert!(test_only[0].message().contains("testOnlyField"));
    Ok(())
}

