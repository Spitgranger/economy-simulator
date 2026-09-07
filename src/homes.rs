//! Homes: the unit that holds money and shops. One or two adults plus their
//! minor children. Tastes and drives used in the goods market are aggregated
//! from the adult members and refreshed whenever membership changes.

use crate::goods::NG;
use crate::ledger::Account;
use crate::people::People;

/// Firms a home knows per good (for education: private schools).
pub const K: usize = 3;
pub const NO_FIRM: u32 = u32::MAX;

pub struct Homes {
    pub n: usize,
    pub active: Vec<bool>,
    pub account: Vec<Account>,
    pub members: Vec<Vec<u32>>, // adult persons
    pub minors: Vec<u8>,
    pub tile: Vec<u16>,         // residential tile, NO_TILE if unhoused

    // aggregated drives (see `refresh`)
    pub appetite: Vec<f64>,
    pub shelter: Vec<f64>,
    pub status: Vec<f64>,
    pub patience: Vec<f64>,
    pub price_sens: Vec<f64>,
    pub search: Vec<f64>,
    pub risk: Vec<f64>,
    pub equity_share: Vec<f64>,
    pub bond_share: Vec<f64>,
    pub value_w: Vec<f64>,    // weight on price-vs-fundamental when trading
    pub momentum_w: Vec<f64>, // weight on the price trend when trading

    pub known: Vec<[[u32; K]; NG]>,
    pub deprivation: Vec<[f64; NG]>,
    pub income_ema: Vec<f64>,

    pub bonds: Vec<i64>,
    pub holdings: Vec<Vec<(u32, u32)>>, // (firm, shares)
    pub assets_value: Vec<i64>,

    pub consumed_month: Vec<[i64; NG]>,
    pub unmet_month: Vec<[i64; NG]>,
    pub income_month: Vec<i64>,
    pub tax_month: Vec<i64>,
    pub transfers_month: Vec<i64>,
    pub dividend_month: Vec<i64>,
    pub interest_month: Vec<i64>,
    pub tuition_month: Vec<i64>,
    pub firms_owned: Vec<u32>,

    free: Vec<u32>,
}

impl Homes {
    pub fn with_capacity(n: usize) -> Homes {
        macro_rules! cols { ($($f:ident),*) => { Homes { n: 0, free: Vec::new(), $($f: Vec::with_capacity(n)),* } } }
        cols!(
            active, account, members, minors, tile, appetite, shelter, status, patience, price_sens, search, risk,
            equity_share, bond_share, value_w, momentum_w, known, deprivation, income_ema, bonds, holdings, assets_value, consumed_month,
            unmet_month, income_month, tax_month, transfers_month, dividend_month, interest_month, tuition_month, firms_owned
        )
    }

    /// New empty home. Reuses a freed slot (and its account) or opens a new account.
    pub fn spawn(&mut self, known: [[u32; K]; NG], mut open_account: impl FnMut() -> Account) -> usize {
        let h = match self.free.pop() {
            Some(h) => h as usize,
            None => {
                self.active.push(false);
                self.account.push(open_account());
                self.members.push(Vec::with_capacity(2));
                self.minors.push(0);
                self.tile.push(crate::city::NO_TILE);
                self.appetite.push(0.0);
                self.shelter.push(0.0);
                self.status.push(0.0);
                self.patience.push(1.0);
                self.price_sens.push(20.0);
                self.search.push(0.5);
                self.risk.push(0.5);
                self.equity_share.push(0.0);
                self.bond_share.push(0.0);
                self.value_w.push(0.5);
                self.momentum_w.push(0.5);
                self.known.push([[NO_FIRM; K]; NG]);
                self.deprivation.push([0.0; NG]);
                self.income_ema.push(0.0);
                self.bonds.push(0);
                self.holdings.push(Vec::new());
                self.assets_value.push(0);
                self.consumed_month.push([0; NG]);
                self.unmet_month.push([0; NG]);
                self.income_month.push(0);
                self.tax_month.push(0);
                self.transfers_month.push(0);
                self.dividend_month.push(0);
                self.interest_month.push(0);
                self.tuition_month.push(0);
                self.firms_owned.push(0);
                self.n += 1;
                self.n - 1
            }
        };
        self.active[h] = true;
        self.members[h].clear();
        self.minors[h] = 0;
        self.tile[h] = crate::city::NO_TILE;
        self.known[h] = known;
        self.deprivation[h] = [0.0; NG];
        self.income_ema[h] = 0.0;
        self.bonds[h] = 0;
        self.holdings[h].clear();
        self.assets_value[h] = 0;
        self.consumed_month[h] = [0; NG];
        self.unmet_month[h] = [0; NG];
        self.income_month[h] = 0;
        self.tax_month[h] = 0;
        self.transfers_month[h] = 0;
        self.dividend_month[h] = 0;
        self.interest_month[h] = 0;
        self.tuition_month[h] = 0;
        self.firms_owned[h] = 0;
        h
    }

    /// Dissolve. Caller has moved out members, minors, money and portfolio.
    pub fn deactivate(&mut self, h: usize) {
        debug_assert!(self.members[h].is_empty() && self.holdings[h].is_empty() && self.bonds[h] == 0);
        self.active[h] = false;
        self.minors[h] = 0;
        self.free.push(h as u32);
    }

    /// Recompute aggregated drives from the adult members.
    pub fn refresh(&mut self, h: usize, ppl: &People) {
        let m = &self.members[h];
        let k = m.len().max(1) as f64;
        let sum = |f: &dyn Fn(usize) -> f64| m.iter().map(|&i| f(i as usize)).sum::<f64>();
        self.appetite[h] = sum(&|i| ppl.appetite[i]);
        let shelter = sum(&|i| ppl.shelter_need[i]);
        self.shelter[h] = if m.len() > 1 { 0.75 * shelter } else { shelter };
        self.status[h] = sum(&|i| ppl.status[i]) / k;
        self.patience[h] = sum(&|i| ppl.patience[i]) / k;
        self.price_sens[h] = sum(&|i| ppl.price_sens[i]) / k;
        self.search[h] = sum(&|i| ppl.search[i]) / k;
        self.risk[h] = sum(&|i| ppl.risk[i]) / k;
        self.equity_share[h] = 0.2 + 0.6 * self.risk[h];
        self.bond_share[h] = 0.3 * (self.patience[h] - 0.5) / 2.5;
        self.value_w[h] = 0.6 + 0.8 * (self.patience[h] - 0.5) / 2.5;
        self.momentum_w[h] = 0.4 * self.risk[h];
    }

    #[inline]
    pub fn has_adults(&self, h: usize) -> bool {
        self.active[h] && !self.members[h].is_empty()
    }

    #[inline]
    pub fn appetite_eff(&self, h: usize) -> f64 {
        self.appetite[h] + 0.5 * self.minors[h] as f64
    }

    #[inline]
    pub fn shelter_eff(&self, h: usize) -> f64 {
        self.shelter[h] + 0.25 * self.minors[h] as f64
    }

    pub fn add_shares(&mut self, h: usize, f: u32, n: u32) {
        if let Some(e) = self.holdings[h].iter_mut().find(|e| e.0 == f) {
            e.1 += n;
        } else {
            self.holdings[h].push((f, n));
        }
    }

    pub fn remove_shares(&mut self, h: usize, f: u32, n: u32) -> u32 {
        if let Some(pos) = self.holdings[h].iter().position(|e| e.0 == f) {
            let have = self.holdings[h][pos].1;
            let take = have.min(n);
            if take == have {
                self.holdings[h].swap_remove(pos);
            } else {
                self.holdings[h][pos].1 -= take;
            }
            take
        } else {
            0
        }
    }
}
