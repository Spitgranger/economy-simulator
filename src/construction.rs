//! Transitional construction: SHELTER inventory stands for building materials.
//! Projects reserve money, buy finite inventory, and divert real shelter-sector
//! labor from daily production. Cancellation leaves a cleared lot: delivered
//! materials and completed work are sunk; only unspent escrow is refunded.
//! Budgets cannot currently be topped up: an exhausted project must be cancelled.
//! Procurement enters firm monthly sales/revenue, but not household daily sales
//! or the consumer price index. Building work is not counted as produced goods.
use crate::{city::Zone, goods::SHELTER, ledger::Account, sim::World};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConstructionStall { Working, Materials, Labor, Budget, Access }
impl ConstructionStall {
    pub fn name(self) -> &'static str { match self { Self::Access => "road access missing", Self::Working => "working", Self::Materials => "materials shortage", Self::Labor => "labor shortage", Self::Budget => "budget exhausted" } }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ConstructionProject {
    pub tile: u16,
    pub zone: Zone,
    pub materials_required: i64,
    pub materials_delivered: i64,
    pub work_required: f64,
    pub work_done: f64,
    pub budget: i64,
    pub spent: i64,
    pub escrow: Account,
    pub stall: ConstructionStall,
}
impl ConstructionProject {
    pub fn progress(&self) -> f64 { (self.work_done / self.work_required).clamp(0.0, 1.0) }
}
impl World {
    pub fn start_construction(&mut self, x: usize, y: usize, zone: Zone) -> Result<(), String> {
        if x >= self.city.w || y >= self.city.h { return Err("tile outside city".into()); }
        let tile = self.city.idx(x, y);
        if self.construction.iter().any(|p| p.tile == tile) { return Err("construction already in progress; cancel first".into()); }
        let current = &self.city.tiles[tile as usize];
        if current.zone == zone { return Err("tile already has this zone".into()); }
        if current.occupants > 0 { return Err("cannot demolish an occupied tile".into()); }
        let budget = zone.cost();
        if self.ledger.balance(self.gov.account) < budget { return Err("treasury cannot fund construction budget".into()); }
        self.city.set(x, y, Zone::Empty);
        if zone == Zone::Empty { return Ok(()); }
        let escrow = self.ledger.open();
        self.ledger.transfer(self.gov.account, escrow, budget, self.day);
        let work_required = match zone { Zone::Residential => 40.0, Zone::Business => 70.0, Zone::School => 100.0, Zone::Park => 20.0, Zone::Road => 15.0, Zone::Empty => unreachable!() };
        self.construction.push(ConstructionProject { tile, zone, materials_required: (work_required * 4.0) as i64, materials_delivered: 0, work_required, work_done: 0.0, budget, spent: 0, escrow, stall: ConstructionStall::Working });
        self.events.log(self.day, "construction", &format!("{} project started at ({x},{y}); budget {budget}", zone.name()));
        Ok(())
    }
    pub fn cancel_construction(&mut self, x: usize, y: usize) -> Result<i64, String> {
        if x >= self.city.w || y >= self.city.h { return Err("tile outside city".into()); }
        let tile = self.city.idx(x, y);
        let index = self.construction.iter().position(|p| p.tile == tile).ok_or("no construction at tile")?;
        let p = self.construction.remove(index);
        let refund = self.ledger.balance(p.escrow);
        if refund > 0 { self.ledger.transfer(p.escrow, self.gov.account, refund, self.day); }
        self.events.log(self.day, "construction", &format!("cancelled at ({x},{y}); refunded {refund}; materials and work sunk, lot cleared"));
        Ok(refund)
    }
    pub(crate) fn construction_day(&mut self) {
        self.construction_labor.resize(self.firms.effective_labor.len(), 0.0);
        self.construction_labor.fill(0.0);
        let mut builders: Vec<usize> = self.firms.active_list.iter().map(|&f| f as usize).filter(|&f| self.firms.good[f] as usize == SHELTER).collect();
        builders.sort_unstable();
        let mut projects = std::mem::take(&mut self.construction);
        for p in &mut projects {
            // Delivery precedes building; materials bound cumulative work.
            let mut reachable_supplier = false;
            for &f in &builders {
                if !self.city.ensure_route(self.firms.tile[f], p.tile) { continue; }
                reachable_supplier = true;
                let price = self.firms.price[f].max(1);
                let units = (p.materials_required - p.materials_delivered).min(self.firms.inventory[f].max(0)).min(self.ledger.balance(p.escrow) / price);
                if units <= 0 { continue; }
                self.city.traffic.add_trip(self.firms.tile[f], p.tile, crate::traffic::TripKind::Freight, ((units + 49) / 50) as u64);
                let cost = units * price;
                self.ledger.transfer(p.escrow, self.firms.account[f], cost, self.day);
                self.firms.inventory[f] -= units;
                self.physical_goods.construction_allocated[SHELTER] += units as i128;
                self.firms.revenue_month[f] += cost;
                self.firms.sales_month[f] += units;
                self.city.built_month += cost;
                p.spent += cost;
                p.materials_delivered += units;
            }
            let material_limit = p.work_required * p.materials_delivered as f64 / p.materials_required as f64;
            let before = p.work_done;
            // A site cannot finish in under four days even with abundant workers.
            let mut daily_limit = p.work_required / 4.0;
            for &f in &builders {
                let available = (self.firms.effective_labor[f] * 0.5 - self.construction_labor[f]).max(0.0);
                let rate = (self.firms.wage_rate[f] as f64 / 7.0).max(1.0);
                let work = available.min(daily_limit).min((material_limit - p.work_done).max(0.0)).min(self.ledger.balance(p.escrow) as f64 / rate);
                if work <= 1e-9 { continue; }
                // Work was capped by affordable cash; clamp a possible floating
                // point roundoff above that integer balance before transferring.
                let cost = ((work * rate).ceil() as i64).min(self.ledger.balance(p.escrow));
                self.ledger.transfer(p.escrow, self.firms.account[f], cost, self.day);
                self.firms.revenue_month[f] += cost;
                self.city.built_month += cost;
                p.spent += cost;
                p.work_done += work;
                daily_limit -= work;
                self.construction_labor[f] += work;
            }
            let cannot_buy = p.materials_delivered < p.materials_required
                && !builders.is_empty()
                && builders.iter().all(|&f| self.firms.price[f].max(1) > self.ledger.balance(p.escrow));
            p.stall = if p.work_done > before { ConstructionStall::Working } else if !reachable_supplier && material_limit <= p.work_done + 1e-9 { ConstructionStall::Access } else if self.ledger.balance(p.escrow) == 0 || cannot_buy { ConstructionStall::Budget } else if material_limit <= p.work_done + 1e-9 { ConstructionStall::Materials } else { ConstructionStall::Labor };
        }
        let mut completed = false;
        for p in projects {
            if p.work_done + 1e-9 >= p.work_required {
                let (x,y) = self.city.xy(p.tile);
                self.city.set(x as usize, y as usize, p.zone);
                completed = true;
                let refund = self.ledger.balance(p.escrow);
                if refund > 0 { self.ledger.transfer(p.escrow, self.gov.account, refund, self.day); }
                self.events.log(self.day, "construction", &format!("{} completed at ({x},{y}); spent {}", p.zone.name(), p.spent));
            } else { self.construction.push(p); }
        }
        if completed {
            // New housing is usable immediately instead of waiting up to a month.
            for h in 0..self.homes.n {
                if self.homes.has_adults(h) && self.homes.tile[h] == crate::city::NO_TILE {
                    self.house(h, crate::city::NO_TILE);
                }
            }
            if let Some(s) = self.public_school {
                if self.firms.tile[s] == crate::city::NO_TILE {
                    self.firms.tile[s] = self.city.tiles.iter().position(|t| t.zone == Zone::School)
                        .map(|i| i as u16).unwrap_or(crate::city::NO_TILE);
                }
            }
            self.city.update_land_values();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Config;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn world() -> World {
        let mut cfg = Config::default();
        cfg.n_hh = 40;
        cfg.n_firms = 10;
        cfg.firm_cap = 20;
        cfg.out_dir = format!("/tmp/econsim-construction-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        cfg.quiet = true;
        let mut w = World::new(cfg).unwrap();
        // These scarcity tests isolate resources from access; provide a spur.
        for x in 0..3 { w.city.set(x, 1, Zone::Road); }
        w.ledger.mint(w.gov.account, 2_000_000);
        for &f in &w.firms.active_list {
            let f = f as usize;
            if w.firms.good[f] as usize == SHELTER {
                w.firms.inventory[f] = 1000;
                w.firms.effective_labor[f] = 100.0;
            }
        }
        w
    }
    #[test]
    fn construction_takes_days_and_only_completed_sites_add_capacity() {
        let mut w = world();
        w.unhouse(0);
        let capacity = w.city.capacity_of(Zone::Residential);
        let total = w.ledger.total();
        let treasury = w.ledger.balance(w.gov.account);
        w.start_construction(0,0,Zone::Residential).unwrap();
        let mut spent = 0;
        for day in 0..4 {
            assert_eq!(w.city.capacity_of(Zone::Residential), capacity);
            assert_eq!(w.homes.tile[0], crate::city::NO_TILE);
            w.construction_day();
            if day < 3 {
                let p = &w.construction[0];
                assert!((p.progress() - (day + 1) as f64 / 4.0).abs() < 1e-8);
                spent = p.spent;
            }
            w.ledger.assert_conserved(w.day);
            assert_eq!(w.ledger.total(), total);
        }
        assert!(w.construction.is_empty());
        assert_ne!(w.homes.tile[0], crate::city::NO_TILE);
        assert_eq!(w.city.capacity_of(Zone::Residential), capacity + 20);
        assert!(treasury - w.ledger.balance(w.gov.account) >= spent);
        assert_eq!(treasury - w.ledger.balance(w.gov.account), w.city.built_month);
    }
    #[test]
    fn shortages_stall_and_cancellation_refunds_only_unspent_cash() {
        let mut w = world();
        for &f in &w.firms.active_list { w.firms.inventory[f as usize] = 0; }
        let treasury = w.ledger.balance(w.gov.account);
        w.start_construction(0,0,Zone::Residential).unwrap();
        w.construction_day();
        assert_eq!(w.construction[0].stall, ConstructionStall::Materials);
        assert_eq!(w.construction[0].work_done, 0.0);
        for &f in &w.firms.active_list {
            let f = f as usize;
            w.firms.inventory[f] = 1000;
            w.firms.effective_labor[f] = 0.0;
        }
        w.construction_day();
        assert_eq!(w.construction[0].stall, ConstructionStall::Labor);
        let spent = w.construction[0].spent;
        assert!(spent > 0);
        let inventory: i64 = w.firms.inventory.iter().sum();
        let refund = w.cancel_construction(0,0).unwrap();
        assert_eq!(refund, Zone::Residential.cost() - spent);
        assert_eq!(w.ledger.balance(w.gov.account), treasury - spent);
        assert_eq!(w.firms.inventory.iter().sum::<i64>(), inventory);
        assert_eq!(w.city.tiles[0].zone, Zone::Empty);
        assert!(w.cancel_construction(0,0).is_err());
        w.ledger.assert_conserved(w.day);
    }
    #[test]
    fn finite_materials_and_shared_labor_bound_all_projects() {
        let mut w = world();
        let builders: Vec<usize> = w.firms.active_list.iter().map(|&f| f as usize).filter(|&f| w.firms.good[f] as usize == SHELTER).collect();
        assert!(!builders.is_empty());
        for &f in &builders { w.firms.inventory[f] = 4; w.firms.effective_labor[f] = 2.0; }
        w.start_construction(0,0,Zone::Residential).unwrap();
        w.start_construction(1,0,Zone::Residential).unwrap();
        assert!(w.start_construction(0,0,Zone::Business).is_err());
        w.construction_day();
        let delivered: i64 = w.construction.iter().map(|p| p.materials_delivered).sum();
        let worked: f64 = w.construction.iter().map(|p| p.work_done).sum();
        assert_eq!(delivered, builders.len() as i64 * 4);
        assert!(worked <= builders.len() as f64 + 1e-8);
        for &f in &builders { assert!(w.construction_labor[f] <= 1.0); assert_eq!(w.firms.inventory[f], 0); }
        assert!(w.start_construction(w.city.w,0,Zone::Park).is_err());
        w.city.tiles[2].occupants = 1;
        assert!(w.start_construction(2,0,Zone::Park).is_err());
        w.ledger.assert_conserved(w.day);
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn unaffordable_materials_stall_without_overspending() {
        let mut cfg = crate::sim::Config::default();
        cfg.n_hh = 40;
        cfg.n_firms = 10;
        cfg.out_dir = format!("/tmp/econsim-construction-budget-{}", std::process::id());
        let mut w = World::new(cfg).unwrap();
        // These scarcity tests isolate resources from access; provide a spur.
        for x in 0..3 { w.city.set(x, 1, Zone::Road); }
        let available = w.ledger.balance(w.gov.account);
        if available > 0 { w.ledger.transfer(w.gov.account, w.homes.account[0], available, 0); }
        assert!(w.start_construction(0,0,Zone::Residential).is_err());
        w.ledger.mint(w.gov.account, Zone::Residential.cost());
        for &f in &w.firms.active_list {
            w.firms.price[f as usize] = Zone::Residential.cost() + 1;
            w.firms.inventory[f as usize] = 1000;
        }
        w.start_construction(0,0,Zone::Residential).unwrap();
        w.construction_day();
        assert_eq!(w.construction[0].stall, ConstructionStall::Budget);
        assert_eq!(w.construction[0].spent, 0);
        assert_eq!(w.cancel_construction(0,0).unwrap(), Zone::Residential.cost());
        w.ledger.assert_conserved(0);
    }
}
