//! Observability: time series for every aggregate, an event log with
//! reasons, and a "watch" log that traces one firm and one household.

use crate::goods::{GOODS, NG};
use std::fs::File;
use std::io::{BufWriter, Write};

#[derive(Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct DailyRow {
    pub day: u32,
    pub price_index: f64,
    pub prices: [f64; NG],
    pub wage: f64,
    pub employed: usize,
    pub unemployment: f64,
    pub output: i64,
    pub sales: i64,
    pub unmet: i64,
    pub inventory: i64,
    pub hh_cash: i64,
    pub firm_cash: i64,
    pub gov_cash: i64,
    pub bank_cash: i64,
    pub money_supply: i64,
    pub loans: i64,
    pub gov_debt: i64,
    pub population: usize,
    pub firms: usize,
}

#[derive(Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct MonthRow {
    pub month: u32,
    pub price_index: f64,
    pub prices: [f64; NG],
    pub wage: f64,
    pub real_wage: f64,
    pub unemployment: f64,
    pub unemp_low_skill: f64,
    pub unemp_high_skill: f64,
    pub output: i64,
    pub sales: i64,
    pub unmet: i64,
    pub inventory: i64,
    pub firms: usize,
    pub firms_by_good: [usize; NG],
    pub employed_by_good: [usize; NG],
    pub bankruptcies: u32,
    pub entries: u32,
    pub exits: u32,
    pub dividends: i64,
    pub markup: f64,
    pub gini: f64,
    pub top10_share: f64,
    pub hh_cash: i64,
    pub firm_cash: i64,
    pub gov_cash: i64,
    pub taxes: i64,
    pub benefits: i64,
    pub universal_dividend: i64,
    pub mean_skill: f64,
    pub pay_p10: i64,
    pub pay_p90: i64,
    pub hires: u32,
    pub fires: u32,
    pub quits: u32,
    pub bank_cash: i64,
    pub money_supply: i64,
    pub loans: i64,
    pub lent: i64,
    pub defaults: u32,
    pub written_off: i64,
    pub policy_rate: f64,
    pub loan_rate: f64,
    pub bond_rate: f64,
    pub inflation: f64,
    pub deposit_interest: i64,
    pub printed: i64,
    pub gov_debt: i64,
    pub bonds_bank: i64,
    pub bond_interest: i64,
    pub bonds_issued: i64,
    pub market_cap: i64,
    pub stock_volume: u32,
    pub stock_turnover: f64, // value traded in the month / market cap
    pub equity_issued: i64,
    pub mean_pref: f64,
    pub incumbent: i64,
    pub vote_shares: [f64; crate::politics::N_PARTIES],
    pub population: usize,
    pub adults: usize,
    pub minors: usize,
    pub retirees: usize,
    pub births: u32,
    pub deaths: u32,
    pub matured: u32,
    pub mean_age: f64,
    pub dependency: f64,
    pub life_expectancy: f64,
    pub mobility_corr: f64,
    pub pensions: i64,
    pub child_benefit: i64,
    pub homes: usize,
    pub couples: usize,
    pub unions: u32,
    pub separations: u32,
    pub pupils_public: u32,
    pub pupils_private: u32,
    pub teachers_public: usize,
    pub teachers_private: usize,
    pub public_quality: f64,
    pub private_quality: f64,
    pub education_spend: i64,
    pub tuition: i64,
    pub hc_new_adults: f64, // -1 when nobody came of age this month
    pub housing_capacity: u32,
    pub homes_housed: u32,
    pub unhoused: u32,
    pub business_slots: u32,
    pub business_used: u32,
    pub school_capacity: u32,
    pub avg_land_value: i64,
    pub rent_revenue: i64,
    pub built: i64,
    pub avg_commute: f64,
    pub immigrants: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Stats {
    #[serde(skip)]
    daily: Option<BufWriter<File>>,
    pub months: Vec<MonthRow>,
    pub last_prices: [f64; NG],
    pub last_wage: f64,
    pub last_cpi: f64,
    pub last_daily: Option<DailyRow>,
    hash: u64,
}

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

impl Stats {
    pub(crate) fn reattach(&mut self, out_dir: &str) -> std::io::Result<()> {
        self.daily = Self::new(out_dir, self.last_wage)?.daily;
        Ok(())
    }

    pub fn new(out_dir: &str, init_wage: f64) -> std::io::Result<Stats> {
        let mut daily = BufWriter::new(File::create(format!("{}/daily.csv", out_dir))?);
        let goods: Vec<String> = GOODS.iter().map(|g| format!("price_{}", g.name)).collect();
        writeln!(
            daily,
            "day,price,{},wage,employed,unemployment,output,sales,unmet,inventory,hh_cash,firm_cash,gov_cash,bank_cash,money_supply,loans,gov_debt,population,firms",
            goods.join(",")
        )?;
        let mut last_prices = [0.0; NG];
        for g in 0..NG {
            last_prices[g] = GOODS[g].init_price as f64;
        }
        Ok(Stats { daily: Some(daily), months: Vec::new(), last_prices, last_wage: init_wage, last_cpi: 100.0, last_daily: None, hash: 0xcbf2_9ce4_8422_2325 })
    }

    pub fn record_day(&mut self, r: &DailyRow) {
        self.last_prices = r.prices;
        self.last_wage = r.wage;
        self.last_cpi = r.price_index;
        self.last_daily = Some(r.clone());
        let prices: Vec<String> = r.prices.iter().map(|p| format!("{:.2}", p)).collect();
        let line = format!(
            "{},{:.3},{},{:.2},{},{:.4},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            r.day, r.price_index, prices.join(","), r.wage, r.employed, r.unemployment, r.output, r.sales,
            r.unmet, r.inventory, r.hh_cash, r.firm_cash, r.gov_cash, r.bank_cash, r.money_supply, r.loans, r.gov_debt, r.population, r.firms
        );
        self.hash = fnv(self.hash, line.as_bytes());
        self.daily.as_mut().expect("stats output attached").write_all(line.as_bytes()).expect("write daily.csv");
    }

    pub fn record_month(&mut self, r: MonthRow) {
        self.months.push(r);
    }

    pub fn hash(&self) -> u64 {
        self.hash
    }

    pub fn write_monthly(&mut self, out_dir: &str) -> std::io::Result<()> {
        let mut w = BufWriter::new(File::create(format!("{}/monthly.csv", out_dir))?);
        let pg: Vec<String> = GOODS.iter().map(|g| format!("price_{}", g.name)).collect();
        let fg: Vec<String> = GOODS.iter().map(|g| format!("firms_{}", g.name)).collect();
        let eg: Vec<String> = GOODS.iter().map(|g| format!("employed_{}", g.name)).collect();
        let vs: Vec<String> = crate::politics::PARTIES.iter().map(|(n, _)| format!("votes_{}", n.to_lowercase())).collect();
        writeln!(w, "month,year,price,{},wage,real_wage,unemployment,unemp_low_skill,unemp_high_skill,output,sales,unmet,inventory,firms,{},{},bankruptcies,entries,exits,dividends,markup,gini,top10_share,hh_cash,firm_cash,gov_cash,taxes,benefits,universal_dividend,mean_skill,pay_p10,pay_p90,hires,fires,quits,bank_cash,money_supply,loans,lent,defaults,written_off,policy_rate,loan_rate,bond_rate,inflation,deposit_interest,printed,gov_debt,bonds_bank,bond_interest,bonds_issued,market_cap,stock_volume,stock_turnover,equity_issued,mean_pref,incumbent,{},population,adults,minors,retirees,births,deaths,matured,mean_age,dependency,life_expectancy,mobility_corr,pensions,child_benefit,homes,couples,unions,separations,pupils_public,pupils_private,teachers_public,teachers_private,public_quality,private_quality,education_spend,tuition,hc_new_adults,housing_capacity,homes_housed,unhoused,business_slots,business_used,school_capacity,avg_land_value,rent_revenue,built,avg_commute,immigrants",
            pg.join(","), fg.join(","), eg.join(","), vs.join(","))?;
        for m in &self.months {
            let pg: Vec<String> = m.prices.iter().map(|p| format!("{:.2}", p)).collect();
            let fg: Vec<String> = m.firms_by_good.iter().map(|v| v.to_string()).collect();
            let eg: Vec<String> = m.employed_by_good.iter().map(|v| v.to_string()).collect();
            let vs: Vec<String> = m.vote_shares.iter().map(|v| format!("{:.4}", v)).collect();
            writeln!(
                w,
                "{},{:.3},{:.3},{},{:.2},{:.4},{:.4},{:.4},{:.4},{},{},{},{},{},{},{},{},{},{},{},{:.4},{:.4},{:.4},{},{},{},{},{},{},{:.4},{},{},{},{},{},{},{},{},{},{},{},{:.4},{:.4},{:.4},{:.4},{},{},{},{},{},{},{},{},{:.4},{},{},{},{},{},{},{},{},{},{},{},{:.2},{:.3},{:.1},{:.3},{},{},{},{},{},{},{},{},{},{},{:.3},{:.3},{},{},{:.3},{},{},{},{},{},{},{},{},{},{:.2},{}",
                m.month,
                m.month as f64 / crate::sim::MONTHS_PER_YEAR as f64,
                m.price_index, pg.join(","), m.wage, m.real_wage, m.unemployment, m.unemp_low_skill, m.unemp_high_skill,
                m.output, m.sales, m.unmet, m.inventory, m.firms, fg.join(","), eg.join(","),
                m.bankruptcies, m.entries, m.exits, m.dividends, m.markup, m.gini, m.top10_share,
                m.hh_cash, m.firm_cash, m.gov_cash, m.taxes, m.benefits, m.universal_dividend,
                m.mean_skill, m.pay_p10, m.pay_p90, m.hires, m.fires, m.quits,
                m.bank_cash, m.money_supply, m.loans, m.lent, m.defaults, m.written_off, m.policy_rate, m.loan_rate, m.bond_rate,
                m.inflation, m.deposit_interest, m.printed, m.gov_debt, m.bonds_bank, m.bond_interest, m.bonds_issued, m.market_cap,
                m.stock_volume, m.stock_turnover, m.equity_issued, m.mean_pref, m.incumbent, vs.join(","),
                m.population, m.adults, m.minors, m.retirees, m.births, m.deaths, m.matured, m.mean_age, m.dependency,
                m.life_expectancy, m.mobility_corr, m.pensions, m.child_benefit,
                m.homes, m.couples, m.unions, m.separations, m.pupils_public, m.pupils_private, m.teachers_public, m.teachers_private,
                m.public_quality, m.private_quality, m.education_spend, m.tuition, m.hc_new_adults,
                m.housing_capacity, m.homes_housed, m.unhoused, m.business_slots, m.business_used, m.school_capacity,
                m.avg_land_value, m.rent_revenue, m.built, m.avg_commute, m.immigrants
            )?;
        }
        self.daily.as_mut().expect("stats output attached").flush()?;
        Ok(())
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct EventLog {
    #[serde(skip)]
    w: Option<BufWriter<File>>,
    pub count: u64,
    /// the most recent events, for a live view
    pub recent: std::collections::VecDeque<(u32, String, String)>,
}

impl EventLog {
    pub(crate) fn reattach(&mut self, path: &str) -> std::io::Result<()> {
        self.w = Self::new(path)?.w;
        Ok(())
    }

    pub fn new(path: &str) -> std::io::Result<EventLog> {
        Ok(EventLog { w: Some(BufWriter::new(File::create(path)?)), count: 0, recent: std::collections::VecDeque::with_capacity(64) })
    }

    pub fn log(&mut self, day: u32, kind: &str, msg: &str) {
        self.count += 1;
        writeln!(self.w.as_mut().expect("event output attached"), "d{:05}\t{:<10}\t{}", day, kind, msg).expect("write event log");
        if self.recent.len() >= 60 {
            self.recent.pop_front();
        }
        self.recent.push_back((day, kind.to_string(), msg.to_string()));
    }

    pub fn flush(&mut self) {
        self.w.as_mut().expect("event output attached").flush().expect("flush event log");
    }
}

/// Gini coefficient and top-decile share of a wealth vector (sorted in place).
pub fn inequality(v: &mut [i64]) -> (f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0);
    }
    v.sort_unstable();
    let n = v.len() as f64;
    let total: i64 = v.iter().sum();
    if total <= 0 {
        return (0.0, 0.0);
    }
    let mut weighted = 0.0;
    for (i, &x) in v.iter().enumerate() {
        weighted += (2.0 * (i as f64 + 1.0) - n - 1.0) * x as f64;
    }
    let gini = weighted / (n * total as f64);
    let top_start = v.len() - (v.len() / 10).max(1);
    let top: i64 = v[top_start..].iter().sum();
    (gini, top as f64 / total as f64)
}

/// Percentile of a vector (sorted in place); q in [0,1].
pub fn percentile(v: &mut [i64], q: f64) -> i64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    let idx = ((v.len() - 1) as f64 * q).round() as usize;
    v[idx]
}
