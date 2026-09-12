//! Controlled market regressions before introducing industrial recipes.
use econsim::{goods::{EDUCATION, FOOD, NG}, Config, World};
use std::{path::PathBuf, sync::atomic::{AtomicU64, Ordering}};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scenario { world: World, path: PathBuf }
impl Scenario {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("econsim-physical-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let world = World::new(Config { n_hh: 40, n_firms: 10, quiet: true,
            out_dir: path.to_string_lossy().into_owned(), ..Config::default() }).unwrap();
        Self { world, path }
    }
}
impl Drop for Scenario {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.path); }
}

#[test]
fn household_purchases_remove_stock_and_pay_suppliers_and_tax() {
    let mut scenario = Scenario::new();
    let w = &mut scenario.world;
    w.gov.policy.sales_tax = [0.2; NG];
    w.firms.effective_labor.fill(0.0);
    w.firms.production_carry.fill(0.0);
    for &f in &w.firms.active_list {
        if !w.firms.is_school(f as usize) { w.firms.inventory[f as usize] = 1000; }
    }
    let stocks = w.firms.inventory.clone();
    let cash: Vec<_> = w.firms.account.iter().map(|&a| w.ledger.balance(a)).collect();
    let household_cash: i64 = w.homes.account.iter().map(|&a| w.ledger.balance(a)).sum();
    let treasury = w.ledger.balance(w.gov.account);
    w.tick_day(); // Day one has no payroll, entry, or monthly policy changes.
    let mut sold = [0; NG];
    let mut receipts = 0;
    for &f in &w.firms.active_list {
        let f = f as usize;
        if w.firms.is_school(f) { continue; }
        let units = stocks[f] - w.firms.inventory[f];
        sold[w.firms.good[f] as usize] += units;
        let received = w.ledger.balance(w.firms.account[f]) - cash[f];
        assert_eq!(received, units * w.firms.price[f]);
        receipts += received;
    }
    assert!(sold[FOOD] > 0, "scenario must exercise purchases");
    let balance = w.physical_goods_balance();
    assert_eq!(balance.day, 1);
    assert_eq!(balance.produced, [0; EDUCATION]);
    for g in 0..EDUCATION {
        assert_eq!(balance.household_consumption[g], sold[g] as i128);
        assert_eq!(balance.opening[g] - balance.closing[g], sold[g] as i128);
    }
    for g in 0..NG {
        if g != EDUCATION {
            assert_eq!(sold[g], w.homes.consumed_month.iter().map(|c| c[g]).sum::<i64>());
        }
    }
    let taxes = w.ledger.balance(w.gov.account) - treasury;
    assert!(taxes > 0);
    let remaining: i64 = w.homes.account.iter().map(|&a| w.ledger.balance(a)).sum();
    assert_eq!(household_cash - remaining, receipts + taxes);
}

#[test]
fn cash_cannot_replace_missing_labor_and_fractional_output_carries_forward() {
    let mut scenario = Scenario::new();
    let w = &mut scenario.world;
    // Empty household wallets isolate production from the goods market.
    for &account in &w.homes.account {
        let amount = w.ledger.balance(account);
        w.ledger.transfer(account, w.gov.account, amount, w.day);
    }
    w.firms.inventory.fill(0);
    w.firms.effective_labor.fill(0.0);
    w.firms.production_carry.fill(0.0);
    let f = *w.firms.active_list.iter().find(|&&f| w.firms.good[f as usize] as usize == FOOD).unwrap() as usize;
    w.ledger.mint(w.firms.account[f], 1_000_000);
    w.tick_day();
    assert_eq!(w.firms.inventory[f], 0);
    w.firms.productivity[f] = 1.0;
    w.firms.effective_labor[f] = 0.5;
    w.tick_day();
    assert_eq!(w.firms.inventory[f], 0);
    w.tick_day();
    assert_eq!(w.firms.inventory[f], 1);
    assert_eq!(w.firms.production_carry[f], 0.0);
}
