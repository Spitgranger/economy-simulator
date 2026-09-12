//! Firms, structure-of-arrays with a free-list so bankrupt slots are reused.
//! A firm is: a sector (good), inventory, cash (a ledger account), labor
//! (employee list, skill-weighted), a production function linear in effective
//! labor, a price rule, a wage/hiring rule and a bankruptcy condition.
//! See `World::firm_decisions`.

use crate::goods::GOODS;
use crate::ledger::Account;

pub const SHARES: u32 = 10_000;
/// Seller id used in the ask book when the firm itself is selling new shares.
pub const FIRM_SELLER: u32 = u32::MAX - 1;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Firms {
    pub active: Vec<bool>,
    pub account: Vec<Account>,
    pub owner: Vec<u32>, // founding home; u32::MAX for the public school
    pub good: Vec<u8>,
    pub public: Vec<bool>,
    pub tile: Vec<u16>, // business tile
    pub pupils: Vec<u32>,       // enrolled this month (education)
    pub quality: Vec<f64>,      // last month's teaching quality (education)

    pub inventory: Vec<i64>,
    pub production_carry: Vec<f64>,
    pub productivity: Vec<f64>, // units per effective worker per day
    pub price: Vec<i64>,        // cents per unit, net of sales tax
    pub wage_rate: Vec<i64>,    // cents per week per unit of skill
    pub employees: Vec<Vec<u32>>,
    pub effective_labor: Vec<f64>, // sum of employee skills, kept in sync
    pub target_workers: Vec<u32>,

    // firm "personality": months of payroll to hold before paying dividends
    pub caution: Vec<f64>,

    // credit (one consolidated loan per firm)
    pub loan_principal: Vec<i64>,
    pub loan_rate: Vec<f64>, // annual, blended
    pub loan_months_left: Vec<u32>,

    // equity: SHARES shares at founding, all to the founder
    pub shares: Vec<u32>,
    pub holders: Vec<Vec<(u32, u32)>>, // (household, shares)
    pub share_price: Vec<f64>,   // last clearing price, cents (fractional)
    pub price_ema: Vec<f64>,     // slow average of the price, the momentum reference
    pub fundamental: Vec<f64>,   // capitalised dividends + book, per share (monthly)
    pub dividend_ema: Vec<f64>,  // smoothed monthly dividend
    pub issue_pending: Vec<u32>, // new shares the firm is offering (equity finance)
    pub sell_book: Vec<Vec<(u32, u32)>>, // session asks: (home, shares); FIRM_SELLER for issuance
    pub bid_book: Vec<Vec<(u32, u32)>>,  // session bids: (home, shares)

    // monthly counters / memory
    pub sales_month: Vec<i64>,
    pub lost_sales_month: Vec<i64>,
    pub revenue_month: Vec<i64>,
    pub wages_paid_month: Vec<i64>,
    pub last_bill: Vec<i64>,
    pub vacancy_unfilled_weeks: Vec<u32>,
    pub distress_weeks: Vec<u32>,
    pub dormant_months: Vec<u32>,
    pub age_months: Vec<u32>,

    free: Vec<u32>,
    pub active_list: Vec<u32>,
}

impl Firms {
    pub fn new(cap: usize, accounts: &[Account]) -> Firms {
        assert_eq!(accounts.len(), cap);
        Firms {
            active: vec![false; cap],
            account: accounts.to_vec(),
            owner: vec![u32::MAX; cap],
            good: vec![0; cap],
            public: vec![false; cap],
            tile: vec![crate::city::NO_TILE; cap],
            pupils: vec![0; cap],
            quality: vec![0.0; cap],
            inventory: vec![0; cap],
            production_carry: vec![0.0; cap],
            productivity: vec![0.0; cap],
            price: vec![0; cap],
            wage_rate: vec![0; cap],
            employees: (0..cap).map(|_| Vec::with_capacity(16)).collect(),
            effective_labor: vec![0.0; cap],
            target_workers: vec![0; cap],
            caution: vec![1.0; cap],
            loan_principal: vec![0; cap],
            loan_rate: vec![0.0; cap],
            loan_months_left: vec![0; cap],
            shares: vec![0; cap],
            holders: (0..cap).map(|_| Vec::with_capacity(8)).collect(),
            share_price: vec![0.0; cap],
            price_ema: vec![0.0; cap],
            fundamental: vec![0.0; cap],
            dividend_ema: vec![0.0; cap],
            issue_pending: vec![0; cap],
            sell_book: (0..cap).map(|_| Vec::new()).collect(),
            bid_book: (0..cap).map(|_| Vec::new()).collect(),
            sales_month: vec![0; cap],
            lost_sales_month: vec![0; cap],
            revenue_month: vec![0; cap],
            wages_paid_month: vec![0; cap],
            last_bill: vec![0; cap],
            vacancy_unfilled_weeks: vec![0; cap],
            distress_weeks: vec![0; cap],
            dormant_months: vec![0; cap],
            age_months: vec![0; cap],
            free: (0..cap as u32).rev().collect(),
            active_list: Vec::with_capacity(cap),
        }
    }

    pub fn n_active(&self) -> usize {
        self.active_list.len()
    }

    /// Activate a free slot. Returns None if the pool is exhausted.
    pub fn activate(
        &mut self,
        owner: u32,
        good: usize,
        productivity: f64,
        price: i64,
        wage_rate: i64,
        target_workers: u32,
        caution: f64,
        seed_capital: i64,
    ) -> Option<usize> {
        let slot = self.free.pop()? as usize;
        self.active[slot] = true;
        self.owner[slot] = owner;
        self.good[slot] = good as u8;
        self.public[slot] = false;
        self.tile[slot] = crate::city::NO_TILE;
        self.pupils[slot] = 0;
        self.quality[slot] = 0.0;
        self.inventory[slot] = 0;
        self.production_carry[slot] = 0.0;
        self.productivity[slot] = productivity;
        self.price[slot] = price.max(1);
        self.wage_rate[slot] = wage_rate.max(1);
        self.employees[slot].clear();
        self.effective_labor[slot] = 0.0;
        self.target_workers[slot] = target_workers;
        self.caution[slot] = caution;
        self.loan_principal[slot] = 0;
        self.loan_rate[slot] = 0.0;
        self.loan_months_left[slot] = 0;
        self.shares[slot] = SHARES;
        self.holders[slot].clear();
        self.holders[slot].push((owner, SHARES));
        self.share_price[slot] = (seed_capital as f64 / SHARES as f64).max(0.01);
        self.price_ema[slot] = self.share_price[slot];
        self.fundamental[slot] = self.share_price[slot];
        self.dividend_ema[slot] = 0.0;
        self.issue_pending[slot] = 0;
        self.sell_book[slot].clear();
        self.bid_book[slot].clear();
        self.sales_month[slot] = 0;
        self.lost_sales_month[slot] = 0;
        self.revenue_month[slot] = 0;
        self.wages_paid_month[slot] = 0;
        self.last_bill[slot] = 0;
        self.vacancy_unfilled_weeks[slot] = 0;
        self.distress_weeks[slot] = 0;
        self.dormant_months[slot] = 0;
        self.age_months[slot] = 0;
        self.active_list.push(slot as u32);
        Some(slot)
    }

    /// Deactivate. Caller must have emptied the ledger account and fired staff.
    pub fn deactivate(&mut self, slot: usize) {
        debug_assert!(self.active[slot]);
        debug_assert!(self.employees[slot].is_empty());
        debug_assert!(self.holders[slot].is_empty());
        self.active[slot] = false;
        self.inventory[slot] = 0;
        self.target_workers[slot] = 0;
        self.effective_labor[slot] = 0.0;
        self.free.push(slot as u32);
        if let Some(pos) = self.active_list.iter().position(|&f| f as usize == slot) {
            self.active_list.swap_remove(pos);
        }
    }

    /// Cost of one unit of output at the posted wage rate. Pay is proportional
    /// to skill and so is output, so skill cancels. For a school the unit is a
    /// seat-month: a month of one teacher's pay spread over a class.
    #[inline]
    pub fn marginal_cost(&self, f: usize) -> f64 {
        if self.good[f] as usize == crate::goods::EDUCATION {
            self.wage_rate[f] as f64 * crate::sim::WEEKS_PER_MONTH as f64 / crate::goods::PRIVATE_CLASS_SIZE as f64
        } else {
            self.wage_rate[f] as f64 / (self.productivity[f] * crate::sim::DAYS_PER_WEEK as f64)
        }
    }

    #[inline]
    pub fn is_school(&self, f: usize) -> bool {
        self.good[f] as usize == crate::goods::EDUCATION
    }

    #[inline]
    pub fn min_skill(&self, f: usize) -> f64 {
        GOODS[self.good[f] as usize].min_skill
    }

    /// Shareholder register: add q shares of firm f to household h.
    pub fn add_holder(&mut self, f: usize, h: u32, q: u32) {
        if let Some(e) = self.holders[f].iter_mut().find(|e| e.0 == h) {
            e.1 += q;
        } else {
            self.holders[f].push((h, q));
        }
    }

    /// Shareholder register: remove up to q shares of firm f from household h.
    pub fn remove_holder(&mut self, f: usize, h: u32, q: u32) -> u32 {
        if let Some(pos) = self.holders[f].iter().position(|e| e.0 == h) {
            let have = self.holders[f][pos].1;
            let take = have.min(q);
            if take == have {
                self.holders[f].swap_remove(pos);
            } else {
                self.holders[f][pos].1 -= take;
            }
            take
        } else {
            0
        }
    }

    #[inline]
    pub fn has_vacancy(&self, f: usize) -> bool {
        self.active[f] && (self.employees[f].len() as u32) < self.target_workers[f]
    }
}
