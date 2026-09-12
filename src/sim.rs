//! The world and its staggered tick loop.
//!
//!   daily   : production, consumption (goods markets, homes shop for their members)
//!   weekly  : public school funding, payroll (with income tax) -> transfer
//!             programs -> layoffs -> hiring / job search
//!   monthly : policy schedule, elections, schooling, firm rules (loans, prices,
//!             wages, hiring, dividends), home network updates, skill and
//!             preference drift, demographics (aging, births, coming of age,
//!             unions, separations, retirement, death), stock market, bond
//!             auction and debt service, bank rates, firm entry and exit
//!
//! Every tick ends with `ledger.assert_conserved`.

use crate::bank::Bank;
use crate::city::{City, Zone, NO_TILE};
use crate::firms::Firms;
use crate::goods::{EDUCATION, FOOD, GOODS, LUXURY, NG, PRIORITY, SHELTER};
use crate::government::{Government, Policy};
use crate::homes::{Homes, K, NO_FIRM as NO_KNOWN};
use crate::ledger::{Account, Ledger};
use crate::people::{People, PersonInit, NO_FIRM, NO_HOME, NO_PARENT};
use crate::rng::Rng;
use crate::stats::{inequality, percentile, DailyRow, EventLog, MonthRow, Stats};

pub const DAYS_PER_WEEK: u32 = 7;
pub const WEEKS_PER_MONTH: u32 = 4;
pub const DAYS_PER_MONTH: u32 = DAYS_PER_WEEK * WEEKS_PER_MONTH;
pub const MONTHS_PER_YEAR: u32 = 12;
pub const DAYS_PER_YEAR: u32 = DAYS_PER_MONTH * MONTHS_PER_YEAR;

#[derive(Clone, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Config {
    pub seed: u64,
    pub days: u32,
    pub n_hh: usize,
    pub n_firms: usize,
    pub firm_cap: usize,
    pub workers_per_firm: usize,
    pub init_wage: i64,
    pub hh_cash: i64,
    pub firm_cash: i64,
    pub out_dir: String,
    pub watch_firm: Option<usize>,
    pub watch_home: Option<usize>,
    pub watch_person: Option<usize>,
    pub quiet: bool,
    pub policy: Policy,
    pub schedule: Vec<(u32, String, String)>,
    pub bank_equity: i64,
    pub credit_enabled: bool,
    pub elections: bool,
    pub stock_trading: bool,
    pub demographics: bool,
    pub first_election_year: u32,
    pub election_every_years: u32,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            seed: 42,
            days: DAYS_PER_YEAR * 20,
            n_hh: 1000,
            n_firms: 100,
            firm_cap: 400,
            workers_per_firm: 8,
            init_wage: 1800,
            hh_cash: 20_000,
            firm_cash: 50_000,
            out_dir: "out".to_string(),
            watch_firm: Some(0),
            watch_home: Some(0),
            watch_person: Some(0),
            quiet: false,
            policy: Policy::default(),
            schedule: Vec::new(),
            bank_equity: 1_000_000,
            credit_enabled: true,
            elections: true,
            stock_trading: true,
            demographics: true,
            first_election_year: 1,
            election_every_years: 4,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct SectorSignal {
    firms: usize,
    avg_price: f64,
    markup: f64,
    unmet_ratio: f64,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct World {
    pub cfg: Config,
    pub day: u32,
    pub ledger: Ledger,
    pub ppl: People,
    pub homes: Homes,
    pub firms: Firms,
    pub gov: Government,
    pub bank: Bank,
    pub city: City,
    pub construction: Vec<crate::construction::ConstructionProject>,
    pub(crate) construction_labor: Vec<f64>,
    pub(crate) physical_goods: crate::physical::PhysicalGoodsBalance,

    rng_hh: Rng,
    rng_firm: Rng,
    rng_goods: Rng,
    rng_labor: Rng,
    rng_entry: Rng,
    pub(crate) rng_politics: Rng,
    pub(crate) rng_finance: Rng,
    pub(crate) rng_demo: Rng,

    order: Vec<u32>,                  // permutation of homes, shuffled per goods-market tick
    pub(crate) job_seekers: Vec<u32>, // scratch list
    vacancies: Vec<u32>,
    pub(crate) scratch: Vec<i64>,
    pub(crate) bond_demand: Vec<i64>, // per home
    base_index: f64,

    pub stats: Stats,
    pub events: EventLog,
    pub(crate) watch: EventLog,

    day_units: [i64; NG],
    day_revenue: [i64; NG],
    day_unmet: i64,
    day_output: i64,

    month_bankrupt: u32,
    month_entries: u32,
    month_exits: u32,
    pub(crate) month_dividends: i64,
    month_hires: u32,
    month_fires: u32,
    month_quits: u32,
    pub(crate) month_stock_volume: u32,
    pub(crate) month_value_traded: i64,
    pub(crate) month_equity_issued: i64,
    pub(crate) market_cap: i64,

    // demographics and schooling bookkeeping
    pub(crate) mobility: Vec<(f64, f64)>,
    pub(crate) deaths_age_sum: f64,
    pub(crate) deaths_total: u64,
    month_births: u32,
    month_deaths: u32,
    month_matured: u32,
    month_unions: u32,
    month_separations: u32,
    month_immigrants: u32,
    pub(crate) month_hc_sum: f64,
    pub(crate) month_tuition: i64,
    pub(crate) public_school: Option<usize>,
    pub(crate) public_pupils: u32,
    pub(crate) private_pupils: u32,
    pub(crate) public_quality: f64,
    pub(crate) month_commute_sum: f64,
}

impl World {
    pub fn new(cfg: Config) -> std::io::Result<World> {
        std::fs::create_dir_all(&cfg.out_dir)?;
        let mut ledger = Ledger::new();
        let mut rng_hh = Rng::new(cfg.seed, 1);
        let mut rng_firm = Rng::new(cfg.seed, 2);
        let rng_goods = Rng::new(cfg.seed, 3);
        let mut rng_labor = Rng::new(cfg.seed, 4);
        let rng_entry = Rng::new(cfg.seed, 5);
        let rng_politics = Rng::new(cfg.seed, 6);
        let rng_finance = Rng::new(cfg.seed, 7);
        let rng_demo = Rng::new(cfg.seed, 8);

        let firm_accounts: Vec<Account> = (0..cfg.firm_cap).map(|_| ledger.open()).collect();
        let gov_account = ledger.open();
        let mut gov = Government::new(gov_account, cfg.policy.clone());
        gov.schedule = cfg.schedule.clone();
        let bank_account = ledger.open();
        let bank = Bank::new(bank_account, cfg.bank_equity, cfg.credit_enabled);
        ledger.mint(bank_account, cfg.bank_equity);

        // Firms by sector share (goods only), then the public school.
        let mut firms = Firms::new(cfg.firm_cap, &firm_accounts);
        let mut sector_of = Vec::with_capacity(cfg.n_firms);
        for g in 0..EDUCATION {
            let n = (GOODS[g].firm_share * cfg.n_firms as f64).floor() as usize;
            for _ in 0..n {
                sector_of.push(g);
            }
        }
        while sector_of.len() < cfg.n_firms {
            sector_of.push(FOOD);
        }
        for (f, &g) in sector_of.iter().enumerate() {
            let productivity = GOODS[g].productivity * rng_firm.uniform(0.85, 1.15);
            let price = (GOODS[g].init_price as f64 * rng_firm.uniform(0.95, 1.05)).round() as i64;
            let wage = (cfg.init_wage as f64 * rng_firm.uniform(0.95, 1.05)).round() as i64;
            let caution = rng_firm.uniform(0.25, 1.0);
            let slot = firms
                .activate(f as u32, g, productivity, price, wage, cfg.workers_per_firm as u32, caution, cfg.firm_cash)
                .expect("firm pool");
            ledger.mint(firms.account[slot], cfg.firm_cash);
        }
        let school = firms.activate(u32::MAX, EDUCATION, 1.0, 0, cfg.init_wage, 0, 1.0, 0).expect("firm pool");
        firms.public[school] = true;
        firms.holders[school].clear();
        firms.shares[school] = 0;
        let mut city = City::starter(24, 16);
        for f in 0..cfg.n_firms {
            let t = city.find_free(Zone::Business, NO_TILE).expect("starter city has room for the first firms");
            city.occupy(t);
            firms.tile[f] = t;
        }
        firms.tile[school] = city.tiles.iter().position(|t| t.zone == Zone::School).map(|i| i as u16).unwrap_or(NO_TILE);
        let firms_of_good: Vec<Vec<u32>> = (0..NG)
            .map(|g| firms.active_list.iter().copied().filter(|&f| firms.good[f as usize] as usize == g && !firms.public[f as usize]).collect())
            .collect();

        // People and their homes: adults aged 18-75, one per home.
        let mut ppl = People::with_capacity(cfg.n_hh * 2);
        let mut homes = Homes::with_capacity(cfg.n_hh * 2);
        for _ in 0..cfg.n_hh {
            let skill = 0.5 + (rng_hh.f64() + rng_hh.f64()) * 0.5;
            let leisure = rng_hh.uniform(0.8, 1.2);
            let patience = rng_hh.uniform(0.5, 3.0);
            let risk = rng_hh.f64();
            let pref = (crate::politics::prior(patience, risk, skill) + rng_hh.uniform(-0.15, 0.15)).clamp(0.0, 1.0);
            let reservation = (cfg.init_wage as f64 * skill * leisure * rng_hh.uniform(0.85, 1.0)) as i64;
            let mut known = [[NO_KNOWN; K]; NG];
            for g in 0..NG {
                let pool = &firms_of_good[g];
                if pool.is_empty() {
                    continue;
                }
                let mut k = 0;
                while k < K && k < pool.len() {
                    let f = pool[rng_hh.below(pool.len())];
                    if !known[g][..k].contains(&f) {
                        known[g][k] = f;
                        k += 1;
                    }
                }
            }
            let age_months = if cfg.demographics { rng_hh.below(57 * 12) as u32 + 18 * 12 } else { 30 * 12 };
            let i = ppl.spawn(PersonInit {
                age_months,
                parent: NO_PARENT,
                home: NO_HOME,
                skill,
                appetite: rng_hh.uniform(1.8, 3.0),
                shelter_need: rng_hh.uniform(0.8, 1.2),
                status: rng_hh.uniform(0.05, 0.3),
                patience,
                price_sens: rng_hh.uniform(10.0, 30.0),
                leisure,
                risk,
                search: rng_hh.f64(),
                family: rng_hh.uniform(0.5, 1.5),
                reservation_wage: reservation,
                pref,
            });
            let h = homes.spawn(known, || ledger.open());
            homes.members[h].push(i as u32);
            ppl.home[i] = h as u32;
            homes.refresh(h, &ppl);
            if let Some(t) = city.find_free(Zone::Residential, NO_TILE) {
                city.occupy(t);
                homes.tile[h] = t;
            }
            homes.income_ema[h] = cfg.init_wage as f64 * skill / DAYS_PER_WEEK as f64;
            ledger.mint(homes.account[h], cfg.hh_cash);
        }
        for f in 0..cfg.n_firms {
            homes.firms_owned[f] += 1;
            homes.add_shares(f, f as u32, crate::firms::SHARES);
            homes.assets_value[f] = cfg.firm_cash;
        }

        // Initial employment: firms take turns picking the next qualified worker.
        let mut pool: Vec<u32> = (0..ppl.n as u32).filter(|&i| ppl.is_worker(i as usize)).collect();
        rng_labor.shuffle(&mut pool);
        let mut taken = vec![false; ppl.n];
        for _ in 0..cfg.workers_per_firm {
            for f in 0..cfg.n_firms {
                let need = firms.min_skill(f);
                if let Some(pos) = pool.iter().position(|&i| !taken[i as usize] && ppl.skill[i as usize] >= need) {
                    let i = pool[pos] as usize;
                    taken[i] = true;
                    firms.employees[f].push(i as u32);
                    firms.effective_labor[f] += ppl.skill[i] * city.work_factor(homes.tile[ppl.home[i] as usize], firms.tile[f]);
                    ppl.employer[i] = f as u32;
                    ppl.pay[i] = (firms.wage_rate[f] as f64 * ppl.skill[i]).round() as i64;
                    ppl.reservation_wage[i] = ppl.pay[i];
                }
            }
        }

        let base_index: f64 = GOODS.iter().map(|g| g.index_weight * g.init_price as f64).sum();
        let stats = Stats::new(&cfg.out_dir, cfg.init_wage as f64)?;
        let mut events = EventLog::new(&format!("{}/events.log", cfg.out_dir))?;
        events.log(0, "policy", &format!("initial policy: {}", gov.policy.describe()));
        if cfg.elections {
            events.log(0, "election", &format!("elections every {} years from year {}; parties {:?}", cfg.election_every_years, cfg.first_election_year, crate::politics::PARTIES));
        }
        let watch = EventLog::new(&format!("{}/watch.log", cfg.out_dir))?;

        let n_homes = homes.n;
        let cfg_market_cap = cfg.n_firms as i64 * cfg.firm_cash;
        let demographics = cfg.demographics;
        let mut w = World {
            cfg,
            day: 0,
            ledger,
            ppl,
            homes,
            firms,
            gov,
            bank,
            city,
            construction: Vec::new(),
            construction_labor: Vec::new(),
            physical_goods: crate::physical::PhysicalGoodsBalance::default(),
            rng_hh,
            rng_firm,
            rng_goods,
            rng_labor,
            rng_entry,
            rng_politics,
            rng_finance,
            rng_demo,
            order: (0..n_homes as u32).collect(),
            job_seekers: Vec::with_capacity(n_homes),
            vacancies: Vec::with_capacity(512),
            scratch: Vec::with_capacity(n_homes),
            bond_demand: vec![0; n_homes],
            base_index,
            stats,
            events,
            watch,
            day_units: [0; NG],
            day_revenue: [0; NG],
            day_unmet: 0,
            day_output: 0,
            month_bankrupt: 0,
            month_entries: 0,
            month_exits: 0,
            month_dividends: 0,
            month_hires: 0,
            month_fires: 0,
            month_quits: 0,
            month_stock_volume: 0,
            month_value_traded: 0,
            month_equity_issued: 0,
            market_cap: cfg_market_cap,
            mobility: Vec::new(),
            deaths_age_sum: 0.0,
            deaths_total: 0,
            month_births: 0,
            month_deaths: 0,
            month_matured: 0,
            month_unions: 0,
            month_separations: 0,
            month_immigrants: 0,
            month_hc_sum: 0.0,
            month_tuition: 0,
            public_school: Some(school),
            public_pupils: 0,
            private_pupils: 0,
            public_quality: 0.0,
            month_commute_sum: 0.0,
        };
        if demographics {
            // couples to start with (about 60% of adults), and children part-way through school
            for _ in 0..40 {
                w.unions();
            }
            let n = w.ppl.n;
            for i in 0..n {
                let age = w.ppl.age_months[i];
                if w.ppl.is_adult(i) && age >= 20 * 12 && age <= 50 * 12 && w.rng_demo.chance(0.4) {
                    let h = w.ppl.home[i] as usize;
                    let child_age = w.rng_demo.below(18 * 12) as u32;
                    let c = w.add_minor(i as u32, h, child_age);
                    let schooled = child_age.saturating_sub(6 * 12) as f64;
                    w.ppl.hc[c] = (0.6 * schooled / crate::education::MONTHS_OF_SCHOOL).min(1.0);
                    w.ppl.children_born[i] += 1;
                }
            }
        }
        Ok(w)
    }

    pub fn run(&mut self) -> std::io::Result<u64> {
        self.ledger.assert_conserved(0);
        for _ in 0..self.cfg.days {
            self.tick_day();
        }
        self.finish()
    }

    /// Write the monthly file and snapshots; flush logs. Safe to call more than once.
    pub fn finish(&mut self) -> std::io::Result<u64> {
        self.stats.write_monthly(&self.cfg.out_dir)?;
        self.dump_agents()?;
        self.events.flush();
        self.watch.flush();
        Ok(self.stats.hash())
    }

    /// Advance one day. Public so an interactive driver can pace the world.
    pub fn tick_day(&mut self) {
        self.day += 1;
        self.physical_goods = crate::physical::PhysicalGoodsBalance::begin(&self.firms, self.day);
        self.day_units = [0; NG];
        self.day_revenue = [0; NG];
        self.day_unmet = 0;
        self.day_output = 0;

        self.construction_day();
        self.produce();
        self.consume();
        if self.day % DAYS_PER_WEEK == 0 {
            self.labor_market();
            if self.cfg.stock_trading {
                self.stock_session();
            }
        }
        if self.day % DAYS_PER_MONTH == 0 {
            self.month_end();
        }
        self.record_day();
        self.ledger.assert_conserved(self.day);
        self.physical_goods.finish(&self.firms);
    }

    /// Producer stock and explained unit flows for the last completed day.
    pub fn physical_goods_balance(&self) -> &crate::physical::PhysicalGoodsBalance {
        &self.physical_goods
    }

    // ------------------------------------------------------------------ helpers

    /// Effective labor person i contributes at firm f: skill, less the commute.
    #[inline]
    pub(crate) fn labor_of(&self, i: usize, f: usize) -> f64 {
        let home = self.ppl.home[i];
        let tile = if home == NO_HOME { NO_TILE } else { self.homes.tile[home as usize] };
        self.ppl.skill[i] * self.city.work_factor(tile, self.firms.tile[f])
    }

    /// Find room for home h on a residential tile near `near`; leaves it unhoused if the city is full.
    pub(crate) fn house(&mut self, h: usize, near: u16) {
        if self.homes.tile[h] != NO_TILE {
            return;
        }
        if let Some(t) = self.city.find_free(Zone::Residential, near) {
            self.city.occupy(t);
            self.homes.tile[h] = t;
        }
    }

    pub(crate) fn unhouse(&mut self, h: usize) {
        let t = self.homes.tile[h];
        if t != NO_TILE {
            self.city.vacate(t);
            self.homes.tile[h] = NO_TILE;
        }
    }

    #[inline]
    fn gross_pay(&self, i: usize, f: usize) -> i64 {
        ((self.firms.wage_rate[f] as f64 * self.ppl.skill[i]).round() as i64).max(self.gov.policy.min_wage).max(1)
    }

    /// Would firm f take person i? Skill floor of the sector, and the worker must
    /// be able to produce at least their own pay (public school: budget-funded).
    #[inline]
    fn acceptable(&self, i: usize, f: usize) -> bool {
        let skill = self.ppl.skill[i];
        if skill < self.firms.min_skill(f) {
            return false;
        }
        if self.firms.public[f] {
            return true;
        }
        let weekly_value = if self.firms.is_school(f) {
            self.firms.price[f] as f64 * crate::goods::PRIVATE_CLASS_SIZE as f64 / WEEKS_PER_MONTH as f64
        } else {
            self.labor_of(i, f) * self.firms.productivity[f] * DAYS_PER_WEEK as f64 * self.firms.price[f] as f64
        };
        self.gross_pay(i, f) as f64 <= weekly_value
    }

    #[inline]
    fn benefit_floor(&self) -> i64 {
        ((self.gov.policy.benefit_rate * self.stats.last_wage) * 1.1) as i64
    }

    // ------------------------------------------------------------------ daily

    fn produce(&mut self) {
        for &f in &self.firms.active_list {
            let f = f as usize;
            if self.firms.is_school(f) {
                continue;
            }
            let carry = self.firms.production_carry[f] + (self.firms.effective_labor[f] - self.construction_labor.get(f).copied().unwrap_or(0.0)).max(0.0) * self.firms.productivity[f];
            let units = carry.floor();
            self.firms.production_carry[f] = carry - units;
            self.firms.inventory[f] += units as i64;
            self.physical_goods.produced[self.firms.good[f] as usize] += units as i128;
            self.day_output += units as i64;
        }
    }

    /// Goods markets. Each home decides how much of each good it wants today
    /// (pooled drives tempered by its cash buffer) in priority order, then picks
    /// among the firms it knows by softmax over relative gross price.
    fn consume(&mut self) {
        let n = self.homes.n;
        if self.order.len() != n {
            self.order.clear();
            self.order.extend(0..n as u32);
        }
        self.rng_goods.shuffle(&mut self.order[..n]);
        for idx in 0..n {
            let h = self.order[idx] as usize;
            if !self.homes.has_adults(h) {
                continue;
            }
            let acc = self.homes.account[h];
            let mut cash = self.ledger.balance(acc);

            let mut p_ref = [0f64; NG];
            for &g in &PRIORITY {
                let mut sum = 0i64;
                let mut cnt = 0i64;
                for &f in &self.homes.known[h][g] {
                    if f == NO_KNOWN || !self.firms.active[f as usize] || self.firms.good[f as usize] as usize != g {
                        continue;
                    }
                    sum += self.gov.gross_price(g, self.firms.price[f as usize]).0;
                    cnt += 1;
                }
                p_ref[g] = if cnt > 0 { sum as f64 / cnt as f64 } else { self.stats.last_prices[g].max(1.0) };
            }
            let appetite = self.homes.appetite_eff(h);
            let shelter_need = self.homes.shelter_eff(h);
            let necessities = appetite * p_ref[FOOD] + shelter_need * p_ref[SHELTER];
            let reserve = self.homes.patience[h] * DAYS_PER_MONTH as f64 * necessities;
            let ratio = if reserve > 0.0 { cash as f64 / reserve } else { 1.0 };
            let mut wanted = [0i64; NG];
            let mut bought = [0i64; NG];

            for &g in &PRIORITY {
                let want = match g {
                    SHELTER => shelter_need * ratio.powf(0.7).clamp(0.5, 1.0) + 0.5 * self.homes.deprivation[h][g],
                    FOOD => appetite * ratio.powf(0.7).clamp(0.3, 1.5) + 0.5 * self.homes.deprivation[h][g],
                    _ => {
                        if ratio < 1.0 {
                            0.0
                        } else {
                            let mpc = 0.5 + 0.4 * (self.homes.status[h] - 0.05) / 0.25;
                            let surplus_income = (self.homes.income_ema[h] - necessities).max(0.0);
                            let excess_cash = (cash as f64 - reserve).max(0.0);
                            let assets = self.homes.assets_value[h].max(0) as f64;
                            // dear money: idle cash is spent more slowly when rates are high
                            let damp = (1.0 - 3.0 * self.bank.policy_rate).clamp(0.25, 1.0);
                            (mpc * surplus_income + 0.01 * damp * excess_cash + 0.0002 * assets) / p_ref[LUXURY]
                        }
                    }
                };
                let mut desired = want.floor() as i64 + if self.rng_goods.chance(want.fract()) { 1 } else { 0 };
                wanted[g] = desired;

                let mut opts: [(u32, i64, i64); K] = [(0, 0, 0); K];
                let mut n_opts = 0usize;
                for &f in &self.homes.known[h][g] {
                    if f == NO_KNOWN {
                        continue;
                    }
                    let fu = f as usize;
                    if self.firms.active[fu] && self.firms.good[fu] as usize == g && self.firms.inventory[fu] > 0 {
                        let (gross, tax) = self.gov.gross_price(g, self.firms.price[fu]);
                        opts[n_opts] = (f, gross, tax);
                        n_opts += 1;
                    }
                }
                let mut w = [0f64; K];
                for j in 0..n_opts {
                    w[j] = (-self.homes.price_sens[h] * (opts[j].1 as f64 / p_ref[g] - 1.0)).exp();
                }
                let mut remaining = n_opts;
                while desired > 0 && remaining > 0 && cash > 0 {
                    let total: f64 = w[..remaining].iter().sum();
                    let mut r = self.rng_goods.f64() * total;
                    let mut j = 0;
                    while j + 1 < remaining {
                        r -= w[j];
                        if r < 0.0 {
                            break;
                        }
                        j += 1;
                    }
                    let (fidx, mut gross, tax) = opts[j];
                    let fi = fidx as usize;
                    if g == SHELTER && self.homes.tile[h] == NO_TILE {
                        gross += gross / 2; // unhoused: the shelter premium goes to the seller
                    }
                    let affordable = cash / gross;
                    let buy = desired.min(self.firms.inventory[fi]).min(affordable);
                    if buy > 0 {
                        let net = buy * (gross - tax);
                        self.ledger.transfer(acc, self.firms.account[fi], net, self.day);
                        if tax > 0 {
                            self.ledger.transfer(acc, self.gov.account, buy * tax, self.day);
                            self.gov.sales_tax_month += buy * tax;
                            self.homes.tax_month[h] += buy * tax;
                        }
                        cash -= buy * gross;
                        self.firms.inventory[fi] -= buy;
                        self.physical_goods.household_consumption[g] += buy as i128;
                        self.firms.sales_month[fi] += buy;
                        self.firms.revenue_month[fi] += net;
                        desired -= buy;
                        bought[g] += buy;
                        self.day_units[g] += buy;
                        self.day_revenue[g] += buy * gross;
                    }
                    if desired > 0 && self.firms.inventory[fi] == 0 {
                        self.firms.lost_sales_month[fi] += desired;
                    }
                    remaining -= 1;
                    opts.swap(j, remaining);
                    w.swap(j, remaining);
                }
                if desired > 0 && n_opts == 0 {
                    if let Some(&f) = self.homes.known[h][g].iter().find(|&&f| f != NO_KNOWN && self.firms.active[f as usize] && self.firms.good[f as usize] as usize == g) {
                        self.firms.lost_sales_month[f as usize] += desired;
                    }
                }
                self.homes.consumed_month[h][g] += bought[g];
                self.homes.unmet_month[h][g] += desired;
                self.day_unmet += desired;
                if g != LUXURY {
                    let base = if g == FOOD { appetite } else { shelter_need };
                    let dep = (self.homes.deprivation[h][g] + (base - bought[g] as f64)).max(0.0);
                    self.homes.deprivation[h][g] = (dep * 0.8).min(base);
                }
            }

            if self.cfg.watch_home == Some(h) && self.day % DAYS_PER_WEEK == 1 {
                self.watch.log(
                    self.day,
                    "home",
                    &format!(
                        "home{} members={:?} minors={} cash={} reserve={:.0} ratio={:.2} wanted={:?} bought={:?} p_ref=[{:.0},{:.0},{:.0}]",
                        h, self.homes.members[h], self.homes.minors[h], cash, reserve, ratio, wanted, bought, p_ref[0], p_ref[1], p_ref[2]
                    ),
                );
            }
        }
    }

    // ----------------------------------------------------------------- weekly

    fn labor_market(&mut self) {
        // 0. Public school payroll is funded from the treasury.
        self.gov.education_last_week = 0;
        if let Some(s) = self.public_school {
            let mut bill = 0i64;
            for k in 0..self.firms.employees[s].len() {
                bill += self.gross_pay(self.firms.employees[s][k] as usize, s);
            }
            let have = self.ledger.balance(self.firms.account[s]);
            let need = (bill - have).max(0);
            let amt = need.min(self.ledger.balance(self.gov.account));
            if amt > 0 {
                self.ledger.transfer(self.gov.account, self.firms.account[s], amt, self.day);
            }
            if amt < need {
                self.gov.unfunded_benefit_weeks += 1;
            }
            self.gov.education_last_week = bill.min(have + amt);
            self.gov.education_month += self.gov.education_last_week;
        }

        // 1. Payroll, with income tax withheld into the treasury; pay lands in the worker's home.
        let income_tax = self.gov.policy.income_tax;
        let mut idx = 0;
        while idx < self.firms.active_list.len() {
            let f = self.firms.active_list[idx] as usize;
            let n = self.firms.employees[f].len();
            if n == 0 {
                idx += 1;
                continue;
            }
            let mut bill = 0i64;
            for k in 0..n {
                bill += self.gross_pay(self.firms.employees[f][k] as usize, f);
            }
            let cash = self.ledger.balance(self.firms.account[f]);
            let short = cash < bill;
            for k in 0..n {
                let i = self.firms.employees[f][k] as usize;
                let gross = self.gross_pay(i, f);
                let paid = if short { cash * gross / bill } else { gross };
                let tax = (paid as f64 * income_tax).floor() as i64;
                let h = self.ppl.home[i] as usize;
                self.ledger.transfer(self.firms.account[f], self.homes.account[h], paid - tax, self.day);
                if tax > 0 {
                    self.ledger.transfer(self.firms.account[f], self.gov.account, tax, self.day);
                    self.gov.income_tax_month += tax;
                    self.homes.tax_month[h] += tax;
                }
                self.ppl.pay[i] = gross;
                self.homes.income_month[h] += paid;
            }
            self.firms.wages_paid_month[f] += bill.min(cash);
            self.firms.last_bill[f] = bill;
            if !short {
                self.firms.distress_weeks[f] = 0;
                idx += 1;
                continue;
            }
            if self.firms.public[f] {
                let per_worker = (bill / n as i64).max(1);
                self.firms.target_workers[f] = self.firms.target_workers[f].min(((cash / per_worker) as u32).max(1));
                idx += 1;
                continue;
            }
            self.firms.distress_weeks[f] += 1;
            if self.firms.distress_weeks[f] >= 3 {
                let reason = format!("cannot meet payroll 3 weeks running: cash {} < bill {} ({} workers)", cash, bill, n);
                self.close_firm(f, "bankrupt", &reason);
                continue;
            }
            let per_worker = (bill / n as i64).max(1);
            let covered = (cash / per_worker) as u32;
            self.firms.target_workers[f] = self.firms.target_workers[f].min(covered.max(1));
            idx += 1;
        }

        // 2. Transfer programs, funded only from the treasury.
        let wage = self.stats.last_wage;
        let unemployed = (self.gov.policy.benefit_rate * wage).round() as i64;
        let basic = (self.gov.policy.basic_income * wage).round() as i64;
        let pension = (self.gov.policy.pension * wage).round() as i64;
        let child = (self.gov.policy.child_benefit * wage).round() as i64;
        self.gov.benefit_last_week = self.pay_program(unemployed, |p, i| p.is_worker(i) && !p.is_employed(i));
        self.gov.basic_income_last_week = self.pay_program(basic, |p, i| p.is_adult(i));
        self.gov.pension_last_week = self.pay_program(pension, |p, i| p.is_adult(i) && p.retired[i]);
        self.gov.child_benefit_last_week = self.pay_child_benefit(child);
        self.gov.benefits_month += self.gov.benefit_last_week + self.gov.basic_income_last_week;
        self.gov.pensions_month += self.gov.pension_last_week;
        self.gov.child_benefit_month += self.gov.child_benefit_last_week;

        // 3. Layoffs down to target (last hired, first fired).
        for ai in 0..self.firms.active_list.len() {
            let f = self.firms.active_list[ai] as usize;
            while self.firms.employees[f].len() as u32 > self.firms.target_workers[f] {
                let i = self.firms.employees[f].pop().unwrap() as usize;
                self.firms.effective_labor[f] -= self.labor_of(i, f);
                self.separate(i);
                self.month_fires += 1;
                if self.cfg.watch_firm == Some(f) || self.cfg.watch_person == Some(i) {
                    self.watch.log(self.day, "fire", &format!("firm{} lays off p{} (target {})", f, i, self.firms.target_workers[f]));
                }
            }
        }

        // 4. Vacancies.
        self.vacancies.clear();
        for &f in &self.firms.active_list {
            if self.firms.has_vacancy(f as usize) {
                self.vacancies.push(f);
            }
        }

        // 5. Unemployed search.
        let n = self.ppl.n;
        let floor = self.benefit_floor();
        self.job_seekers.clear();
        for i in 0..n {
            if self.ppl.is_worker(i) && !self.ppl.is_employed(i) {
                self.job_seekers.push(i as u32);
            }
        }
        self.rng_labor.shuffle(&mut self.job_seekers);
        for k in 0..self.job_seekers.len() {
            let i = self.job_seekers[k] as usize;
            if self.vacancies.is_empty() {
                self.ppl.unemployed_weeks[i] += 1;
                continue;
            }
            let tries = 1 + (self.ppl.search[i] * 4.0) as usize;
            let mut best: Option<(usize, i64)> = None;
            for _ in 0..tries {
                if self.vacancies.is_empty() {
                    break;
                }
                let vi = self.rng_labor.below(self.vacancies.len());
                let f = self.vacancies[vi] as usize;
                if !self.firms.has_vacancy(f) {
                    self.vacancies.swap_remove(vi);
                    continue;
                }
                let offer = self.gross_pay(i, f);
                if offer >= self.ppl.reservation_wage[i].max(floor) && self.acceptable(i, f) && best.map_or(true, |(_, b)| offer > b) {
                    best = Some((f, offer));
                }
            }
            if let Some((f, _)) = best {
                self.hire(i, f);
                self.month_hires += 1;
            } else {
                self.ppl.unemployed_weeks[i] += 1;
            }
        }

        // 6. Employed search.
        if !self.vacancies.is_empty() {
            for i in 0..n {
                if !self.ppl.is_employed(i) || !self.rng_labor.chance(0.1 * self.ppl.search[i]) {
                    continue;
                }
                if self.vacancies.is_empty() {
                    break;
                }
                let vi = self.rng_labor.below(self.vacancies.len());
                let f = self.vacancies[vi] as usize;
                if !self.firms.has_vacancy(f) {
                    self.vacancies.swap_remove(vi);
                    continue;
                }
                let old = self.ppl.employer[i] as usize;
                let offer = self.gross_pay(i, f);
                if old != f && offer * 100 > self.ppl.pay[i] * 105 && self.acceptable(i, f) {
                    self.remove_from_firm(i);
                    self.hire(i, f);
                    self.month_quits += 1;
                }
            }
        }

        // 7. Vacancy memory.
        for &f in &self.firms.active_list {
            let f = f as usize;
            if self.firms.has_vacancy(f) {
                self.firms.vacancy_unfilled_weeks[f] += 1;
            } else {
                self.firms.vacancy_unfilled_weeks[f] = 0;
            }
        }
    }

    /// Pay `amount` to the home of every eligible person, pro rata if the treasury is short.
    fn pay_program(&mut self, amount: i64, eligible: impl Fn(&People, usize) -> bool) -> i64 {
        if amount <= 0 {
            return 0;
        }
        let units = (0..self.ppl.n).filter(|&i| eligible(&self.ppl, i)).count() as i64;
        if units == 0 {
            return 0;
        }
        let treasury = self.ledger.balance(self.gov.account);
        let per = amount.min(treasury / units);
        if per < amount {
            self.gov.unfunded_benefit_weeks += 1;
        }
        if per <= 0 {
            return 0;
        }
        let mut total = 0;
        for i in 0..self.ppl.n {
            if eligible(&self.ppl, i) {
                let h = self.ppl.home[i] as usize;
                self.ledger.transfer(self.gov.account, self.homes.account[h], per, self.day);
                self.homes.transfers_month[h] += per;
                self.homes.income_month[h] += per;
                total += per;
            }
        }
        total
    }

    fn pay_child_benefit(&mut self, amount: i64) -> i64 {
        if amount <= 0 {
            return 0;
        }
        let units: i64 = (0..self.homes.n).filter(|&h| self.homes.has_adults(h)).map(|h| self.homes.minors[h] as i64).sum();
        if units == 0 {
            return 0;
        }
        let per = amount.min(self.ledger.balance(self.gov.account) / units);
        if per < amount {
            self.gov.unfunded_benefit_weeks += 1;
        }
        if per <= 0 {
            return 0;
        }
        let mut total = 0;
        for h in 0..self.homes.n {
            if self.homes.has_adults(h) && self.homes.minors[h] > 0 {
                let amt = per * self.homes.minors[h] as i64;
                self.ledger.transfer(self.gov.account, self.homes.account[h], amt, self.day);
                self.homes.transfers_month[h] += amt;
                self.homes.income_month[h] += amt;
                total += amt;
            }
        }
        total
    }

    fn hire(&mut self, i: usize, f: usize) {
        self.firms.employees[f].push(i as u32);
        self.firms.effective_labor[f] += self.labor_of(i, f);
        self.ppl.employer[i] = f as u32;
        self.ppl.pay[i] = self.gross_pay(i, f);
        self.ppl.reservation_wage[i] = self.ppl.pay[i];
        self.ppl.unemployed_weeks[i] = 0;
        if self.cfg.watch_firm == Some(f) || self.cfg.watch_person == Some(i) {
            self.watch.log(self.day, "hire", &format!("firm{} ({}) hires p{} (skill {:.2}) at {}", f, GOODS[self.firms.good[f] as usize].name, i, self.ppl.skill[i], self.ppl.pay[i]));
        }
    }

    fn separate(&mut self, i: usize) {
        self.ppl.employer[i] = NO_FIRM;
        self.ppl.unemployed_weeks[i] = 0;
        self.ppl.reservation_wage[i] = self.ppl.pay[i].max(1);
    }

    fn close_firm(&mut self, f: usize, kind: &str, reason: &str) {
        let staff = std::mem::take(&mut self.firms.employees[f]);
        for &i in &staff {
            self.separate(i as usize);
        }
        self.firms.employees[f] = staff;
        self.firms.employees[f].clear();
        self.firms.effective_labor[f] = 0.0;
        self.city.vacate(self.firms.tile[f]);
        self.firms.tile[f] = NO_TILE;
        let owner = self.firms.owner[f] as usize;
        self.write_off(f);
        self.clear_shareholders(f);
        if owner < self.homes.n {
            self.homes.firms_owned[owner] = self.homes.firms_owned[owner].saturating_sub(1);
        }
        if kind == "bankrupt" {
            self.month_bankrupt += 1;
        } else {
            self.month_exits += 1;
        }
        self.events.log(
            self.day,
            kind,
            &format!(
                "firm{} ({}, age {}m, owner home{}, price {}, wage rate {}, inventory {}) closed: {}",
                f, GOODS[self.firms.good[f] as usize].name, self.firms.age_months[f], owner, self.firms.price[f],
                self.firms.wage_rate[f], self.firms.inventory[f], reason
            ),
        );
        if !self.firms.is_school(f) {
            self.physical_goods.closure_losses[self.firms.good[f] as usize] += self.firms.inventory[f] as i128;
        }
        self.firms.deactivate(f);
    }

    // ---------------------------------------------------------------- monthly

    fn month_end(&mut self) {
        let month = self.day / DAYS_PER_MONTH;

        for change in self.gov.apply_schedule(month) {
            let desc = self.gov.policy.describe();
            self.events.log(self.day, "policy", &format!("{} -> {}", change, desc));
        }
        if self.gov.unfunded_benefit_weeks > 0 {
            self.events.log(self.day, "treasury", &format!("transfers underfunded in {} program-week(s) this month", self.gov.unfunded_benefit_weeks));
            self.gov.unfunded_benefit_weeks = 0;
        }
        if self.cfg.elections {
            let first = self.cfg.first_election_year * MONTHS_PER_YEAR;
            let every = (self.cfg.election_every_years * MONTHS_PER_YEAR).max(1);
            if month >= first && (month - first) % every == 0 {
                self.election(month);
            }
        }

        self.enroll_and_teach();

        // Sector signals (private firms only), computed before counters reset.
        let mut sig = [SectorSignal::default(); NG];
        let mut sum_price = [0.0; NG];
        let mut sum_markup = [0.0; NG];
        let mut lost = [0i64; NG];
        let mut sold = [0i64; NG];
        let mut sum_wage = 0.0;
        let mut n_private = 0usize;
        for &f in &self.firms.active_list {
            let f = f as usize;
            if self.firms.public[f] {
                continue;
            }
            let g = self.firms.good[f] as usize;
            sig[g].firms += 1;
            sum_price[g] += self.firms.price[f] as f64;
            sum_markup[g] += self.firms.price[f] as f64 / self.firms.marginal_cost(f).max(1e-9);
            lost[g] += self.firms.lost_sales_month[f];
            sold[g] += self.firms.sales_month[f];
            sum_wage += self.firms.wage_rate[f] as f64;
            n_private += 1;
        }
        let avg_wage_rate = if n_private > 0 { sum_wage / n_private as f64 } else { self.cfg.init_wage as f64 };
        let mut markup_all = 0.0;
        for g in 0..NG {
            if sig[g].firms > 0 {
                sig[g].avg_price = sum_price[g] / sig[g].firms as f64;
                sig[g].markup = sum_markup[g] / sig[g].firms as f64;
                sig[g].unmet_ratio = if sold[g] + lost[g] > 0 { lost[g] as f64 / (sold[g] + lost[g]) as f64 } else { 0.0 };
                markup_all += sum_markup[g];
            } else {
                sig[g].avg_price = self.stats.last_prices[g];
                sig[g].markup = 1.5;
                // an empty education sector is only an opportunity if the public system is weak
                sig[g].unmet_ratio = if g == EDUCATION { (1.0 - self.public_quality).clamp(0.0, 1.0) } else { 1.0 };
            }
        }
        let markup_all = if n_private > 0 { markup_all / n_private as f64 } else { 1.0 };

        self.firm_decisions();
        self.household_monthly();
        if self.cfg.demographics {
            let (b, d, m) = self.demographics();
            self.month_births = b;
            self.month_deaths = d;
            self.month_matured = m;
            self.month_unions = self.unions();
            self.month_separations = self.separations();
            let unemployment = self.stats.last_daily.as_ref().map(|d| d.unemployment).unwrap_or(1.0);
            self.month_immigrants = self.immigration(unemployment);
        }
        self.city_month();
        self.portfolio_month();
        self.government_finance(month);
        self.bank_month();
        self.firm_entry(&sig, avg_wage_rate);
        self.record_month(month, markup_all);
        self.gov.reset_month();
        self.bank.reset_month();
    }

    /// The classic adaptive rule set: inventory low -> raise price, hire;
    /// inventory high -> cut price, shed a worker. Wages chase vacancies.
    /// Schools count seats as inventory (reset monthly at enrollment).
    fn firm_decisions(&mut self) {
        let mut ai = 0;
        while ai < self.firms.active_list.len() {
            let f = self.firms.active_list[ai] as usize;
            if self.firms.public[f] {
                self.public_school_decisions(f);
                ai += 1;
                continue;
            }
            let n = self.firms.employees[f].len() as u32;
            let demand = self.firms.sales_month[f] + self.firms.lost_sales_month[f];
            let inv = self.firms.inventory[f];
            let mc = self.firms.marginal_cost(f);
            self.firms.issue_pending[f] = 0; // last month's unsold offer lapses
            let low = inv * 4 < demand;
            let high = inv > demand;
            let old_price = self.firms.price[f];
            let old_wage = self.firms.wage_rate[f];
            let old_target = self.firms.target_workers[f];
            let mut why: &str = "hold";

            if !self.service_loan(f) {
                let reason = format!("cannot service loan of {} cents (cash {})", self.firms.loan_principal[f], self.ledger.balance(self.firms.account[f]));
                self.close_firm(f, "bankrupt", &reason);
                continue;
            }

            if low {
                self.firms.target_workers[f] = self.firms.target_workers[f].max(n + 1);
                if (self.firms.price[f] as f64) < mc * 1.5 {
                    let p = (self.firms.price[f] as f64 * (1.0 + self.rng_firm.uniform(0.0, 0.05))).ceil() as i64;
                    self.firms.price[f] = p.max(self.firms.price[f] + 1);
                }
                why = "inventory low";
            } else if high {
                self.firms.target_workers[f] = n.saturating_sub(1);
                if self.firms.price[f] as f64 > mc * 1.02 {
                    let p = (self.firms.price[f] as f64 * (1.0 - self.rng_firm.uniform(0.0, 0.05))).floor() as i64;
                    self.firms.price[f] = p.max((mc * 1.02).ceil() as i64).max(1);
                }
                why = "inventory high";
            } else {
                self.firms.target_workers[f] = n;
            }

            if n < self.firms.target_workers[f] && self.firms.vacancy_unfilled_weeks[f] >= 2 {
                let w = (self.firms.wage_rate[f] as f64 * (1.0 + self.rng_firm.uniform(0.0, 0.05))).ceil() as i64;
                self.firms.wage_rate[f] = w.max(self.firms.wage_rate[f] + 1);
            } else if high && n == self.firms.target_workers[f] + 1 {
                let w = (self.firms.wage_rate[f] as f64 * (1.0 - self.rng_firm.uniform(0.0, 0.02))).floor() as i64;
                self.firms.wage_rate[f] = w.max(1);
            }

            let per_worker = if n > 0 { (self.firms.last_bill[f] / n as i64).max(1) } else { self.firms.wage_rate[f].max(self.gov.policy.min_wage) };
            let mut cash = self.ledger.balance(self.firms.account[f]);
            let mut borrowed = 0;
            let wanted_cash = WEEKS_PER_MONTH as i64 * self.firms.target_workers[f] as i64 * per_worker;
            if low && cash < wanted_cash && (self.firms.price[f] as f64) > mc * (1.0 + self.bank.loan_rate) {
                borrowed = self.borrow(f, wanted_cash - cash);
                cash += borrowed;
            }

            let affordable = (cash / (2 * per_worker)) as u32;
            if self.firms.target_workers[f] > affordable {
                // ask the stock market for the shortfall before shrinking
                let shortfall = 2 * per_worker * (self.firms.target_workers[f] - affordable) as i64;
                self.request_equity(f, shortfall);
                self.firms.target_workers[f] = affordable;
                why = "cash squeeze";
            }

            let monthly_bill = WEEKS_PER_MONTH as i64 * n as i64 * per_worker;
            let buffer = ((self.firms.caution[f] * monthly_bill as f64) as i64).max(2 * per_worker);
            let mut dividend = 0;
            if self.firms.loan_principal[f] == 0 && cash > buffer {
                dividend = cash - buffer;
            }
            self.distribute_dividend(f, dividend);

            if self.cfg.watch_firm == Some(f) {
                self.watch.log(
                    self.day,
                    "firm",
                    &format!(
                        "firm{} ({}) {}: sales={} lost={} inv={} workers={} target {}->{} price {}->{} (mc {:.1}) wage rate {}->{} cash={} borrowed={} loan={} dividend={} share={}",
                        f, GOODS[self.firms.good[f] as usize].name, why, self.firms.sales_month[f], self.firms.lost_sales_month[f], inv, n,
                        old_target, self.firms.target_workers[f], old_price, self.firms.price[f], mc, old_wage, self.firms.wage_rate[f],
                        cash - dividend, borrowed, self.firms.loan_principal[f], dividend, self.firms.share_price[f] as i64
                    ),
                );
            }

            self.firms.age_months[f] += 1;
            let dormant = self.firms.sales_month[f] == 0 && n == 0;
            self.firms.dormant_months[f] = if dormant { self.firms.dormant_months[f] + 1 } else { 0 };
            self.firms.sales_month[f] = 0;
            self.firms.lost_sales_month[f] = 0;
            self.firms.revenue_month[f] = 0;
            self.firms.wages_paid_month[f] = 0;

            if self.firms.dormant_months[f] >= 3 {
                self.close_firm(f, "exit", "no staff and no sales for 3 months");
                continue;
            }
            ai += 1;
        }
    }

    fn random_firm_of(rng: &mut Rng, firms: &Firms, g: usize) -> Option<u32> {
        for _ in 0..16 {
            if firms.active_list.is_empty() {
                return None;
            }
            let f = firms.active_list[rng.below(firms.active_list.len())];
            if firms.good[f as usize] as usize == g && !firms.public[f as usize] {
                return Some(f);
            }
        }
        firms.active_list.iter().copied().find(|&f| firms.good[f as usize] as usize == g && !firms.public[f as usize])
    }

    fn household_monthly(&mut self) {
        // people: reservation wages, skills, politics
        for i in 0..self.ppl.n {
            if !self.ppl.is_adult(i) {
                continue;
            }
            if self.ppl.is_worker(i) && !self.ppl.is_employed(i) && self.ppl.unemployed_weeks[i] >= WEEKS_PER_MONTH {
                self.ppl.reservation_wage[i] = ((self.ppl.reservation_wage[i] as f64) * 0.9).floor().max(1.0) as i64;
            }
            if self.ppl.is_employed(i) {
                self.ppl.skill[i] += 0.003 * (1.5 - self.ppl.skill[i]);
            } else if self.ppl.is_worker(i) {
                self.ppl.skill[i] -= 0.002 * (self.ppl.skill[i] - 0.5);
            }
            self.update_preference(i);
        }
        // homes: shopping networks, income smoothing, counters
        for h in 0..self.homes.n {
            if !self.homes.has_adults(h) {
                continue;
            }
            let mut known = self.homes.known[h];
            for g in 0..NG {
                for k in 0..K {
                    let f = known[g][k];
                    // A closed firm's slot can now belong to another sector.
                    if f == NO_KNOWN || !self.firms.active[f as usize] || self.firms.good[f as usize] as usize != g {
                        known[g][k] = NO_KNOWN;
                        if let Some(r) = Self::random_firm_of(&mut self.rng_hh, &self.firms, g) {
                            if !known[g].contains(&r) {
                                known[g][k] = r;
                            }
                        }
                    }
                }
                if self.rng_hh.chance(0.25) {
                    if let Some(r) = Self::random_firm_of(&mut self.rng_hh, &self.firms, g) {
                        if !known[g].contains(&r) {
                            let mut m = 0;
                            for k in 1..K {
                                if known[g][k] != NO_KNOWN && (known[g][m] == NO_KNOWN || self.firms.price[known[g][k] as usize] > self.firms.price[known[g][m] as usize]) {
                                    m = k;
                                }
                            }
                            if known[g][m] == NO_KNOWN || self.firms.price[r as usize] * 100 < self.firms.price[known[g][m] as usize] * 99 {
                                known[g][m] = r;
                            }
                        }
                    }
                }
                if self.rng_hh.chance(0.05 * self.homes.search[h]) {
                    if let Some(r) = Self::random_firm_of(&mut self.rng_hh, &self.firms, g) {
                        if !known[g].contains(&r) {
                            let k = self.rng_hh.below(K);
                            known[g][k] = r;
                        }
                    }
                }
                if g != EDUCATION && self.homes.unmet_month[h][g] > 0 && self.rng_hh.chance(0.5) {
                    if let Some(r) = Self::random_firm_of(&mut self.rng_hh, &self.firms, g) {
                        if !known[g].contains(&r) {
                            let mut m = 0;
                            for k in 1..K {
                                if known[g][k] == NO_KNOWN || (known[g][m] != NO_KNOWN && self.firms.inventory[known[g][k] as usize] < self.firms.inventory[known[g][m] as usize]) {
                                    m = k;
                                }
                            }
                            known[g][m] = r;
                        }
                    }
                }
            }
            self.homes.known[h] = known;

            if self.cfg.watch_home == Some(h) {
                self.watch.log(
                    self.day,
                    "home-month",
                    &format!(
                        "home{} members={:?} minors={} consumed={:?} unmet={:?} income={} (dividends {} interest {}) tax={} transfers={} tuition={} cash={} assets={} bonds={} holdings={:?}",
                        h, self.homes.members[h], self.homes.minors[h], self.homes.consumed_month[h], self.homes.unmet_month[h], self.homes.income_month[h],
                        self.homes.dividend_month[h], self.homes.interest_month[h], self.homes.tax_month[h], self.homes.transfers_month[h],
                        self.homes.tuition_month[h], self.ledger.balance(self.homes.account[h]), self.homes.assets_value[h], self.homes.bonds[h], self.homes.holdings[h]
                    ),
                );
            }
            let daily_income = self.homes.income_month[h] as f64 / DAYS_PER_MONTH as f64;
            self.homes.income_ema[h] = 0.5 * self.homes.income_ema[h] + 0.5 * daily_income;
            self.homes.consumed_month[h] = [0; NG];
            self.homes.unmet_month[h] = [0; NG];
            self.homes.income_month[h] = 0;
            self.homes.tax_month[h] = 0;
            self.homes.transfers_month[h] = 0;
            self.homes.dividend_month[h] = 0;
            self.homes.interest_month[h] = 0;
            self.homes.tuition_month[h] = 0;
        }
        for ai in 0..self.firms.active_list.len() {
            let f = self.firms.active_list[ai] as usize;
            let mut total = 0.0;
            for k in 0..self.firms.employees[f].len() {
                total += self.labor_of(self.firms.employees[f][k] as usize, f);
            }
            self.firms.effective_labor[f] = total;
        }
    }

    /// Monthly city step: the player's builds, land values, ground rent,
    /// housing the unhoused, commute statistics.
    fn city_month(&mut self) {
        // builds ordered since last month
        let builds = std::mem::take(&mut self.gov.pending_builds);
        for (x, y, z) in builds {
            if let Err(reason) = self.start_construction(x, y, Zone::from_u8(z)) {
                self.events.log(self.day, "construction", &format!("order at ({x},{y}) rejected: {reason}"));
            }
        }
        if let Some(s) = self.public_school {
            if self.firms.tile[s] == NO_TILE || self.city.tiles[self.firms.tile[s] as usize].zone != Zone::School {
                self.firms.tile[s] = self.city.tiles.iter().position(|t| t.zone == Zone::School).map(|i| i as u16).unwrap_or(NO_TILE);
            }
        }
        // house the unhoused if room appeared
        for h in 0..self.homes.n {
            if self.homes.has_adults(h) && self.homes.tile[h] == NO_TILE {
                self.house(h, NO_TILE);
            }
        }
        self.city.update_land_values();
        // ground rent to the treasury
        let rate = self.gov.policy.ground_rent.max(0.0);
        let mut rent_total = 0i64;
        if rate > 0.0 {
            for h in 0..self.homes.n {
                if !self.homes.has_adults(h) || self.homes.tile[h] == NO_TILE {
                    continue;
                }
                let due = (self.city.tiles[self.homes.tile[h] as usize].land_value as f64 * rate) as i64;
                let pay = due.min(self.ledger.balance(self.homes.account[h]));
                if pay > 0 {
                    self.ledger.transfer(self.homes.account[h], self.gov.account, pay, self.day);
                    self.homes.tax_month[h] += pay;
                    rent_total += pay;
                }
            }
            for ai in 0..self.firms.active_list.len() {
                let f = self.firms.active_list[ai] as usize;
                if self.firms.public[f] || self.firms.tile[f] == NO_TILE {
                    continue;
                }
                let due = (self.city.tiles[self.firms.tile[f] as usize].land_value as f64 * rate * 2.0) as i64;
                let pay = due.min(self.ledger.balance(self.firms.account[f]));
                if pay > 0 {
                    self.ledger.transfer(self.firms.account[f], self.gov.account, pay, self.day);
                    rent_total += pay;
                }
            }
        }
        self.gov.rent_month = rent_total;
        self.city.rent_month = rent_total;
        // average commute of the employed
        let mut d_sum = 0.0;
        let mut n = 0usize;
        for &f in &self.firms.active_list {
            let f = f as usize;
            for &i in &self.firms.employees[f] {
                let home = self.ppl.home[i as usize];
                let tile = if home == NO_HOME { NO_TILE } else { self.homes.tile[home as usize] };
                d_sum += self.city.distance(tile, self.firms.tile[f]) as f64;
                n += 1;
            }
        }
        self.month_commute_sum = if n > 0 { d_sum / n as f64 } else { 0.0 };
    }

    /// Treasury above the reserve is returned equally to every adult (into their home).
    pub(crate) fn universal_dividend(&mut self, reserve: i64) {
        let share = self.gov.policy.surplus_dividend;
        if share <= 0.0 {
            return;
        }
        let treasury = self.ledger.balance(self.gov.account);
        let excess = treasury - reserve;
        if excess <= 0 {
            return;
        }
        let adults = (0..self.ppl.n).filter(|&i| self.ppl.is_adult(i)).count() as i64;
        if adults == 0 {
            return;
        }
        let per = ((excess as f64 * share) as i64) / adults;
        if per <= 0 {
            return;
        }
        for i in 0..self.ppl.n {
            if self.ppl.is_adult(i) {
                let h = self.ppl.home[i] as usize;
                self.ledger.transfer(self.gov.account, self.homes.account[h], per, self.day);
                self.homes.transfers_month[h] += per;
                self.homes.income_month[h] += per;
            }
        }
        self.gov.dividend_paid_month += per * adults;
    }

    /// Entry: wealthy, risk-tolerant homes found firms, choosing the sector by
    /// softmax over profit (markup) and unmet demand (stockouts, weak public schooling).
    fn firm_entry(&mut self, sig: &[SectorSignal; NG], avg_wage_rate: f64) {
        let active = self.firms.n_active();
        let best = sig.iter().map(|s| s.markup * (1.0 + 10.0 * s.unmet_ratio)).fold(0.0, f64::max);
        let signal = if best > 1.05 * 1.1 { 1.0 } else { 0.15 };
        let cap = (active / 20).max(1);
        let startup = (WEEKS_PER_MONTH as f64 * 4.0 * avg_wage_rate) as i64;
        let n = self.homes.n;
        self.job_seekers.clear();
        self.job_seekers.extend(0..n as u32);
        self.rng_entry.shuffle(&mut self.job_seekers);
        let mut founded = 0usize;
        for k in 0..n {
            if founded >= cap {
                break;
            }
            let h = self.job_seekers[k] as usize;
            if !self.homes.has_adults(h) {
                continue;
            }
            let cash = self.ledger.balance(self.homes.account[h]);
            if cash < startup * 3 / 2 {
                continue;
            }
            if !self.rng_entry.chance(self.homes.risk[h] * 0.05 * signal) {
                continue;
            }
            let mut w = [0f64; NG];
            for g in 0..NG {
                w[g] = (4.0 * sig[g].markup * (1.0 + 10.0 * sig[g].unmet_ratio)).exp();
            }
            let total: f64 = w.iter().sum();
            let mut r = self.rng_entry.f64() * total;
            let mut g = 0;
            while g + 1 < NG {
                r -= w[g];
                if r < 0.0 {
                    break;
                }
                g += 1;
            }
            let Some(tile) = self.city.find_free(Zone::Business, self.homes.tile[h]) else {
                self.events.log(self.day, "city", "a founder found no business lot free; zone more business land");
                break;
            };
            let seed = startup.max(cash / 2);
            let productivity = GOODS[g].productivity * self.rng_entry.uniform(0.85, 1.15);
            let price = (sig[g].avg_price * self.rng_entry.uniform(0.95, 1.05)).round().max(1.0) as i64;
            let wage = (avg_wage_rate * self.rng_entry.uniform(1.0, 1.05)).round() as i64;
            let caution = self.rng_entry.uniform(0.25, 1.0);
            let Some(f) = self.firms.activate(h as u32, g, productivity, price, wage, 4, caution, seed) else {
                self.events.log(self.day, "entry", "firm pool exhausted; entry skipped");
                break;
            };
            self.ledger.transfer(self.homes.account[h], self.firms.account[f], seed, self.day);
            self.homes.firms_owned[h] += 1;
            self.homes.add_shares(h, f as u32, crate::firms::SHARES);
            self.city.occupy(tile);
            self.firms.tile[f] = tile;
            founded += 1;
            self.month_entries += 1;
            self.events.log(
                self.day,
                "entry",
                &format!(
                    "home{} founds firm{} in {} with {} cents (price {}, wage rate {}, prod {:.2}); sector signal: markup {:.2}, unmet {:.1}%",
                    h, f, GOODS[g].name, seed, price, wage, productivity, sig[g].markup, sig[g].unmet_ratio * 100.0
                ),
            );
        }
    }

    // ------------------------------------------------------------- recording

    fn dump_agents(&self) -> std::io::Result<()> {
        use std::io::Write;
        let mut w = std::io::BufWriter::new(std::fs::File::create(format!("{}/people.csv", self.cfg.out_dir))?);
        writeln!(w, "id,home,age,parent,retired,hc,private_school,skill,employer,sector,pay,reservation_wage,unemployed_weeks,pref,appetite,shelter_need,status,patience,price_sens,leisure,risk,search,family")?;
        for i in 0..self.ppl.n {
            if !self.ppl.alive[i] {
                continue;
            }
            let (emp, sector) = if self.ppl.is_employed(i) {
                let f = self.ppl.employer[i] as usize;
                (f as i64, GOODS[self.firms.good[f] as usize].name)
            } else {
                (-1, "-")
            };
            writeln!(
                w,
                "{},{},{},{},{},{:.3},{},{:.3},{},{},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.2},{:.3},{:.3},{:.3},{:.3}",
                i, self.ppl.home[i], self.ppl.age_months[i] / 12, if self.ppl.parent[i] == NO_PARENT { -1 } else { self.ppl.parent[i] as i64 },
                self.ppl.retired[i] as u8, self.ppl.hc[i], self.ppl.private_school[i] as u8, self.ppl.skill[i], emp, sector, self.ppl.pay[i],
                self.ppl.reservation_wage[i], self.ppl.unemployed_weeks[i], self.ppl.pref[i], self.ppl.appetite[i], self.ppl.shelter_need[i],
                self.ppl.status[i], self.ppl.patience[i], self.ppl.price_sens[i], self.ppl.leisure[i], self.ppl.risk[i], self.ppl.search[i], self.ppl.family[i]
            )?;
        }
        let mut w = std::io::BufWriter::new(std::fs::File::create(format!("{}/homes.csv", self.cfg.out_dir))?);
        writeln!(w, "id,adults,minors,tile,cash,assets,bonds,n_holdings,income_ema,appetite,shelter,status,patience,risk,firms_owned")?;
        for h in 0..self.homes.n {
            if !self.homes.active[h] {
                continue;
            }
            writeln!(
                w,
                "{},{},{},{},{},{},{},{},{:.1},{:.2},{:.2},{:.3},{:.2},{:.2},{}",
                h, self.homes.members[h].len(), self.homes.minors[h], self.homes.tile[h] as i32, self.ledger.balance(self.homes.account[h]), self.homes.assets_value[h],
                self.homes.bonds[h], self.homes.holdings[h].len(), self.homes.income_ema[h], self.homes.appetite[h], self.homes.shelter[h],
                self.homes.status[h], self.homes.patience[h], self.homes.risk[h], self.homes.firms_owned[h]
            )?;
        }
        let mut w = std::io::BufWriter::new(std::fs::File::create(format!("{}/firms.csv", self.cfg.out_dir))?);
        writeln!(w, "id,sector,public,owner,cash,loan,share_price,n_holders,inventory,pupils,quality,workers,effective_labor,target_workers,price,wage_rate,marginal_cost,productivity,caution,age_months")?;
        for &f in &self.firms.active_list {
            let f = f as usize;
            writeln!(
                w,
                "{},{},{},{},{},{},{:.2},{},{},{},{:.2},{},{:.2},{},{},{},{:.2},{:.3},{:.2},{}",
                f, GOODS[self.firms.good[f] as usize].name, self.firms.public[f] as u8, self.firms.owner[f] as i64, self.ledger.balance(self.firms.account[f]),
                self.firms.loan_principal[f], self.firms.share_price[f], self.firms.holders[f].len(), self.firms.inventory[f], self.firms.pupils[f],
                self.firms.quality[f], self.firms.employees[f].len(), self.firms.effective_labor[f], self.firms.target_workers[f],
                self.firms.price[f], self.firms.wage_rate[f], self.firms.marginal_cost(f), self.firms.productivity[f], self.firms.caution[f], self.firms.age_months[f]
            )?;
        }
        Ok(())
    }
}

struct Agg {
    price_index: f64,
    prices: [f64; NG],
    wage: f64,
    employed: usize,
    employed_by_good: [usize; NG],
    firms_by_good: [usize; NG],
    inventory: i64,
    hh_cash: i64,
    firm_cash: i64,
    gov_cash: i64,
    bank_cash: i64,
    population: usize,
    adults: usize,
    workers: usize,
    minors: usize,
    retirees: usize,
    age_sum: f64,
    homes: usize,
    couples: usize,
    teachers_public: usize,
    teachers_private: usize,
}

impl World {
    fn aggregates(&self) -> Agg {
        let mut a = Agg {
            price_index: 0.0,
            prices: self.stats.last_prices,
            wage: self.stats.last_wage,
            employed: 0,
            employed_by_good: [0; NG],
            firms_by_good: [0; NG],
            inventory: 0,
            hh_cash: 0,
            firm_cash: 0,
            gov_cash: self.ledger.balance(self.gov.account),
            bank_cash: self.ledger.balance(self.bank.account),
            population: 0,
            adults: 0,
            workers: 0,
            minors: 0,
            retirees: 0,
            age_sum: 0.0,
            homes: 0,
            couples: 0,
            teachers_public: 0,
            teachers_private: 0,
        };
        for i in 0..self.ppl.n {
            if !self.ppl.alive[i] {
                continue;
            }
            a.population += 1;
            a.age_sum += self.ppl.age_months[i] as f64 / 12.0;
            if self.ppl.is_adult(i) {
                a.adults += 1;
                if self.ppl.retired[i] {
                    a.retirees += 1;
                } else {
                    a.workers += 1;
                }
            } else {
                a.minors += 1;
            }
        }
        for h in 0..self.homes.n {
            if self.homes.has_adults(h) {
                a.homes += 1;
                if self.homes.members[h].len() >= 2 {
                    a.couples += 1;
                }
            }
        }
        let mut pay_sum = 0i64;
        for &f in &self.firms.active_list {
            let f = f as usize;
            let g = self.firms.good[f] as usize;
            let n = self.firms.employees[f].len();
            a.employed += n;
            a.employed_by_good[g] += n;
            if !self.firms.public[f] {
                a.firms_by_good[g] += 1;
            }
            if self.firms.is_school(f) {
                if self.firms.public[f] {
                    a.teachers_public += n;
                } else {
                    a.teachers_private += n;
                }
            }
            for &i in &self.firms.employees[f] {
                pay_sum += self.ppl.pay[i as usize];
            }
            if !self.firms.is_school(f) {
                a.inventory += self.firms.inventory[f];
            }
            a.firm_cash += self.ledger.balance(self.firms.account[f]);
        }
        for g in 0..NG {
            if self.day_units[g] > 0 {
                a.prices[g] = self.day_revenue[g] as f64 / self.day_units[g] as f64;
            }
        }
        // tuition as the education "price": average private tuition, if any
        let mut t_sum = 0i64;
        let mut t_n = 0i64;
        for &f in &self.firms.active_list {
            let f = f as usize;
            if self.firms.is_school(f) && !self.firms.public[f] {
                t_sum += self.firms.price[f];
                t_n += 1;
            }
        }
        if t_n > 0 {
            a.prices[EDUCATION] = t_sum as f64 / t_n as f64;
        }
        let basket: f64 = (0..NG).map(|g| GOODS[g].index_weight * a.prices[g]).sum();
        a.price_index = basket / self.base_index * 100.0;
        if a.employed > 0 {
            a.wage = pay_sum as f64 / a.employed as f64;
        }
        a.hh_cash = self.ledger.total() - a.firm_cash - a.gov_cash - a.bank_cash;
        a
    }

    fn record_day(&mut self) {
        let a = self.aggregates();
        let row = DailyRow {
            day: self.day,
            price_index: a.price_index,
            prices: a.prices,
            wage: a.wage,
            employed: a.employed,
            unemployment: 1.0 - a.employed as f64 / a.workers.max(1) as f64,
            output: self.day_output,
            sales: self.day_units.iter().sum(),
            unmet: self.day_unmet,
            inventory: a.inventory,
            hh_cash: a.hh_cash,
            firm_cash: a.firm_cash,
            gov_cash: a.gov_cash,
            bank_cash: a.bank_cash,
            money_supply: self.ledger.money_supply(),
            loans: self.bank.loans_total,
            gov_debt: self.gov.debt,
            population: a.population,
            firms: self.firms.n_active(),
        };
        self.stats.record_day(&row);
    }

    fn record_month(&mut self, month: u32, markup: f64) {
        let a = self.aggregates();
        self.scratch.clear();
        for h in 0..self.homes.n {
            if self.homes.has_adults(h) {
                self.scratch.push(self.ledger.balance(self.homes.account[h]));
            }
        }
        let (gini, top10) = inequality(&mut self.scratch);
        self.scratch.clear();
        for i in 0..self.ppl.n {
            if self.ppl.is_employed(i) {
                self.scratch.push(self.ppl.pay[i]);
            }
        }
        let pay_p10 = percentile(&mut self.scratch, 0.1);
        let pay_p90 = percentile(&mut self.scratch, 0.9);
        let (mut low_n, mut low_u, mut high_n, mut high_u) = (0usize, 0usize, 0usize, 0usize);
        let mut skill_sum = 0.0;
        for i in 0..self.ppl.n {
            if !self.ppl.is_worker(i) {
                continue;
            }
            skill_sum += self.ppl.skill[i];
            let u = !self.ppl.is_employed(i) as usize;
            if self.ppl.skill[i] < 1.0 {
                low_n += 1;
                low_u += u;
            } else {
                high_n += 1;
                high_u += u;
            }
        }
        let mut priv_q = 0.0;
        for &f in &self.firms.active_list {
            let f = f as usize;
            if self.firms.is_school(f) && !self.firms.public[f] {
                priv_q += self.firms.quality[f] * self.firms.pupils[f] as f64;
            }
        }
        let private_quality = if self.private_pupils > 0 { priv_q / self.private_pupils as f64 } else { 0.0 };
        let row = MonthRow {
            month,
            price_index: a.price_index,
            prices: a.prices,
            wage: a.wage,
            real_wage: a.wage / a.price_index.max(1e-9),
            unemployment: 1.0 - a.employed as f64 / a.workers.max(1) as f64,
            unemp_low_skill: if low_n > 0 { low_u as f64 / low_n as f64 } else { 0.0 },
            unemp_high_skill: if high_n > 0 { high_u as f64 / high_n as f64 } else { 0.0 },
            output: self.day_output,
            sales: self.day_units.iter().sum(),
            unmet: self.day_unmet,
            inventory: a.inventory,
            firms: self.firms.n_active(),
            firms_by_good: a.firms_by_good,
            employed_by_good: a.employed_by_good,
            bankruptcies: self.month_bankrupt,
            entries: self.month_entries,
            exits: self.month_exits,
            dividends: self.month_dividends,
            markup,
            gini,
            top10_share: top10,
            hh_cash: a.hh_cash,
            firm_cash: a.firm_cash,
            gov_cash: a.gov_cash,
            taxes: self.gov.tax_month(),
            benefits: self.gov.benefits_month,
            universal_dividend: self.gov.dividend_paid_month,
            mean_skill: skill_sum / a.workers.max(1) as f64,
            pay_p10,
            pay_p90,
            hires: self.month_hires,
            fires: self.month_fires,
            quits: self.month_quits,
            bank_cash: a.bank_cash,
            money_supply: self.ledger.money_supply(),
            loans: self.bank.loans_total,
            lent: self.bank.lent_month,
            defaults: self.bank.defaults_month,
            written_off: self.bank.written_off_month,
            policy_rate: self.bank.policy_rate,
            loan_rate: self.bank.loan_rate,
            bond_rate: self.gov.bond_rate,
            inflation: self.bank.inflation,
            deposit_interest: self.bank.deposit_interest_month,
            printed: self.gov.printed_month,
            gov_debt: self.gov.debt,
            bonds_bank: self.bank.bonds,
            bond_interest: self.gov.bond_interest_month,
            bonds_issued: self.gov.issued_month,
            market_cap: self.market_cap,
            stock_volume: self.month_stock_volume,
            stock_turnover: if self.market_cap > 0 { self.month_value_traded as f64 / self.market_cap as f64 } else { 0.0 },
            equity_issued: self.month_equity_issued,
            mean_pref: (0..self.ppl.n).filter(|&i| self.ppl.is_adult(i)).map(|i| self.ppl.pref[i]).sum::<f64>() / a.adults.max(1) as f64,
            incumbent: self.gov.incumbent.map(|k| k as i64).unwrap_or(-1),
            vote_shares: self.gov.vote_shares,
            population: a.population,
            adults: a.adults,
            minors: a.minors,
            retirees: a.retirees,
            births: self.month_births,
            deaths: self.month_deaths,
            matured: self.month_matured,
            mean_age: a.age_sum / a.population.max(1) as f64,
            dependency: (a.minors + a.retirees) as f64 / a.workers.max(1) as f64,
            life_expectancy: if self.deaths_total > 0 { self.deaths_age_sum / self.deaths_total as f64 } else { 0.0 },
            mobility_corr: self.mobility_correlation(),
            pensions: self.gov.pensions_month,
            child_benefit: self.gov.child_benefit_month,
            homes: a.homes,
            couples: a.couples,
            unions: self.month_unions,
            separations: self.month_separations,
            pupils_public: self.public_pupils,
            pupils_private: self.private_pupils,
            teachers_public: a.teachers_public,
            teachers_private: a.teachers_private,
            public_quality: self.public_quality,
            private_quality,
            education_spend: self.gov.education_month,
            tuition: self.month_tuition,
            hc_new_adults: if self.month_matured > 0 { self.month_hc_sum / self.month_matured as f64 } else { -1.0 },
            housing_capacity: self.city.capacity_of(Zone::Residential),
            homes_housed: self.city.occupants_of(Zone::Residential),
            unhoused: (0..self.homes.n).filter(|&h| self.homes.has_adults(h) && self.homes.tile[h] == NO_TILE).count() as u32,
            business_slots: self.city.capacity_of(Zone::Business),
            business_used: self.city.occupants_of(Zone::Business),
            school_capacity: self.city.capacity_of(Zone::School),
            avg_land_value: self.city.tiles.iter().map(|t| t.land_value).sum::<i64>() / self.city.tiles.len() as i64,
            rent_revenue: self.gov.rent_month,
            built: self.city.built_month,
            avg_commute: self.month_commute_sum,
            immigrants: self.month_immigrants,
        };
        self.city.built_month = 0;
        if !self.cfg.quiet && month % MONTHS_PER_YEAR == 0 {
            eprintln!(
                "year {:>3} | cpi {:>6.1} | unemp {:>5.1}% | gini {:.3} | pop {:>5} homes {:>4} (couples {:>3}) kids {:>3} ret {:>3} | school: pub {:>3}/{:>2}t q={:.2} priv {:>3}/{:>2}t q={:.2} | gov {} | mobility r={:.2}",
                month / MONTHS_PER_YEAR, a.price_index, row.unemployment * 100.0, gini, row.population, row.homes, row.couples, row.minors, row.retirees,
                row.pupils_public, row.teachers_public, row.public_quality, row.pupils_private, row.teachers_private, private_quality,
                self.gov.incumbent.map(|k| crate::politics::PARTIES[k].0).unwrap_or("-"), row.mobility_corr
            );
        }
        self.stats.record_month(row);
        self.month_bankrupt = 0;
        self.month_entries = 0;
        self.month_exits = 0;
        self.month_dividends = 0;
        self.month_hires = 0;
        self.month_fires = 0;
        self.month_quits = 0;
        self.month_stock_volume = 0;
        self.month_value_traded = 0;
        self.month_equity_issued = 0;
        self.month_births = 0;
        self.month_deaths = 0;
        self.month_matured = 0;
        self.month_unions = 0;
        self.month_separations = 0;
        self.month_immigrants = 0;
        self.month_hc_sum = 0.0;
        self.month_tuition = 0;
    }
}

#[cfg(test)]
mod construction_production_tests {
    use super::*;
    #[test]
    fn construction_work_reduces_daily_factory_output() {
        let mut cfg = Config::default();
        cfg.n_hh = 40;
        cfg.n_firms = 10;
        cfg.out_dir = format!("/tmp/econsim-construction-output-{}", std::process::id());
        let mut w = World::new(cfg).unwrap();
        w.ledger.mint(w.gov.account, 1_000_000);
        for &f in &w.firms.active_list {
            let f = f as usize;
            w.firms.inventory[f] = 1000;
            w.firms.effective_labor[f] = 8.0;
            w.firms.production_carry[f] = 0.0;
        }
        w.start_construction(0, 0, Zone::Residential).unwrap();
        w.construction_day();
        let before = w.firms.inventory.clone();
        assert!(w.construction_labor.iter().sum::<f64>() > 0.0);
        w.produce();
        for &f in &w.firms.active_list {
            let f = f as usize;
            if w.firms.is_school(f) { continue; }
            let expected = ((8.0 - w.construction_labor[f]) * w.firms.productivity[f]).floor() as i64;
            assert_eq!(w.firms.inventory[f] - before[f], expected);
        }
    }

    #[test]
    fn physical_closure_loss_survives_slot_reuse_in_another_sector() {
        let mut cfg = Config::default();
        cfg.n_hh = 40;
        cfg.n_firms = 10;
        cfg.quiet = true;
        cfg.out_dir = format!("/tmp/econsim-physical-closure-{}", std::process::id());
        let out = cfg.out_dir.clone();
        let mut w = World::new(cfg).unwrap();
        let f = w.firms.active_list.iter().map(|&f| f as usize)
            .find(|&f| w.firms.good[f] as usize == FOOD).unwrap();
        w.firms.inventory[f] = 73;
        w.physical_goods = crate::physical::PhysicalGoodsBalance::begin(&w.firms, 1);
        w.close_firm(f, "exit", "physical accounting test");
        let reused = w.firms.activate(0, SHELTER, 1.0, 1, 1, 1, 0.0, 1).unwrap();
        assert_eq!(reused, f);
        assert_eq!(w.firms.inventory[f], 0);
        w.physical_goods.finish(&w.firms);
        assert_eq!(w.physical_goods.closure_losses[FOOD], 73);
        assert_eq!(w.physical_goods.closure_losses[SHELTER], 0);
        w.ledger.assert_conserved(1);
        drop(w);
        std::fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn stale_shopping_link_cannot_sell_a_reused_firms_new_good() {
        let mut cfg = Config::default();
        cfg.n_hh = 40;
        cfg.n_firms = 10;
        cfg.quiet = true;
        cfg.out_dir = format!("/tmp/econsim-stale-shopping-{}", std::process::id());
        let out = cfg.out_dir.clone();
        let mut w = World::new(cfg).unwrap();
        let f = w.firms.active_list.iter().map(|&f| f as usize)
            .find(|&f| w.firms.good[f] as usize == FOOD).unwrap();
        w.close_firm(f, "exit", "shopping network regression");
        assert_eq!(w.firms.activate(0, SHELTER, 1.0, 1, 1, 1, 0.0, 1), Some(f));
        w.firms.inventory[f] = 1000;
        for h in 0..w.homes.n {
            w.homes.known[h] = [[NO_KNOWN; K]; NG];
            w.homes.known[h][FOOD][0] = f as u32;
            w.ledger.mint(w.homes.account[h], 100_000);
        }
        w.physical_goods = crate::physical::PhysicalGoodsBalance::begin(&w.firms, 1);
        w.consume();
        assert_eq!(w.firms.inventory[f], 1000, "shelter cannot satisfy a food order");
        assert_eq!(w.firms.sales_month[f], 0);
        assert_eq!(w.firms.lost_sales_month[f], 0, "food shortages are not shelter demand");
        assert!(w.day_unmet > 0);
        w.physical_goods.finish(&w.firms);
        w.household_monthly();
        for h in 0..w.homes.n {
            if !w.homes.has_adults(h) { continue; }
            for g in 0..NG {
                for &supplier in &w.homes.known[h][g] {
                    if supplier != NO_KNOWN {
                        assert!(w.firms.active[supplier as usize]);
                        assert_eq!(w.firms.good[supplier as usize] as usize, g);
                    }
                }
            }
        }
        drop(w);
        std::fs::remove_dir_all(out).unwrap();
    }
}
