//! Demographics: aging, birth, coming of age (schooling sets skill),
//! partnering and separation, retirement, death and inheritance. Runs
//! monthly. Every money movement is a ledger transfer; slots and accounts of
//! the dead and of dissolved homes are recycled.

use crate::people::{PersonInit, ADULT_AGE, NO_HOME, NO_PARENT, RETIRE_AGE};
use crate::sim::{World, DAYS_PER_MONTH};

/// Yearly probability of dying at a given age: Gompertz, ~0.04% at 18, 3.5% at 70, 19% at 90.
pub fn mortality_yearly(age_years: f64) -> f64 {
    (0.0005 * (0.085 * (age_years - 20.0)).exp()).min(0.9)
}

impl World {
    /// Monthly demographic tick. Returns (births, deaths, matured).
    pub(crate) fn demographics(&mut self) -> (u32, u32, u32) {
        let mut births = 0;
        let mut deaths = 0;
        let mut matured = 0;
        let n = self.ppl.n;
        for i in 0..n {
            if !self.ppl.alive[i] {
                continue;
            }
            self.ppl.age_months[i] += 1;
            let age = self.ppl.age_months[i];
            let p_death = mortality_yearly(age as f64 / 12.0) / 12.0;
            if self.rng_demo.chance(p_death) {
                self.die(i);
                deaths += 1;
                continue;
            }
            if age == ADULT_AGE {
                self.come_of_age(i);
                matured += 1;
            }
            if age == RETIRE_AGE && !self.ppl.retired[i] {
                self.retire(i);
            }
            if age >= 20 * 12 && age <= 40 * 12 && self.ppl.is_adult(i) {
                let h = self.ppl.home[i] as usize;
                if self.homes.minors[h] < 4 {
                    let cash = self.ledger.balance(self.homes.account[h]) as f64;
                    let reserve = self.homes.patience[h] * DAYS_PER_MONTH as f64 * self.necessities_daily(h);
                    let secure = if cash >= reserve { 1.0 } else { 0.6 };
                    if self.rng_demo.chance(0.0045 * self.ppl.family[i] * secure) {
                        self.birth(i);
                        births += 1;
                    }
                }
            }
        }
        (births, deaths, matured)
    }

    fn child_init(parent: u32, home: u32, age_months: u32) -> PersonInit {
        PersonInit {
            age_months,
            parent,
            home,
            skill: 0.0,
            appetite: 0.0,
            shelter_need: 0.0,
            status: 0.0,
            patience: 1.0,
            price_sens: 20.0,
            leisure: 1.0,
            risk: 0.5,
            search: 0.5,
            family: 1.0,
            reservation_wage: 0,
            pref: 0.5,
        }
    }

    pub(crate) fn add_minor(&mut self, parent: u32, home: usize, age_months: u32) -> usize {
        let c = self.ppl.spawn(Self::child_init(parent, home as u32, age_months));
        self.homes.minors[home] += 1;
        c
    }

    fn birth(&mut self, parent: usize) {
        let h = self.ppl.home[parent] as usize;
        let c = self.add_minor(parent as u32, h, 0);
        self.ppl.children_born[parent] = self.ppl.children_born[parent].saturating_add(1);
        if self.cfg.watch_home == Some(h) {
            self.watch.log(self.day, "birth", &format!("home{}: p{} has a child, p{} (now {} minors)", h, parent, c, self.homes.minors[h]));
        }
    }

    /// At 18: a new home of one. Personality half inherited, half drawn; skill
    /// from schooling (human capital) with the parent's skill and wealth
    /// mattering more the less schooling there was; a cash gift from the old home.
    fn come_of_age(&mut self, i: usize) {
        let old_home = self.ppl.home[i] as usize;
        let p = self.ppl.parent[i] as usize;
        let has_parent = self.ppl.parent[i] != NO_PARENT && self.ppl.alive[p];
        let blend = |own: f64, parent: f64, has: bool| if has { 0.5 * own + 0.5 * parent } else { own };
        let (p_skill, p_patience, p_risk, p_pref, p_family) = if has_parent {
            (self.ppl.skill[p], self.ppl.patience[p], self.ppl.risk[p], self.ppl.pref[p], self.ppl.family[p])
        } else {
            (1.0, 1.75, 0.5, 0.5, 1.0)
        };
        let hc = self.ppl.hc[i];
        let wealth = {
            let cash = self.ledger.balance(self.homes.account[old_home]) as f64;
            let reserve = (self.homes.patience[old_home] * DAYS_PER_MONTH as f64 * self.necessities_daily(old_home)).max(1.0);
            (cash / reserve).clamp(0.0, 2.0) / 2.0
        };
        let avg_wage = self.stats.last_wage;
        let known = self.homes.known[old_home];
        let rng = &mut self.rng_demo;
        let noise = (rng.f64() + rng.f64() - 1.0) * 0.2;
        let skill = (0.5 + 0.55 * hc + 0.3 * (p_skill - 0.5) * (1.0 - 0.6 * hc) + 0.15 * wealth * (1.0 - hc) + noise).clamp(0.5, 1.5);
        let patience = blend(rng.uniform(0.5, 3.0), p_patience, has_parent);
        let risk = blend(rng.f64(), p_risk, has_parent);
        let prior = crate::politics::prior(patience, risk, skill);
        let pref = (blend(prior, p_pref, has_parent) + rng.uniform(-0.15, 0.15)).clamp(0.0, 1.0);
        let leisure = rng.uniform(0.8, 1.2);
        let init = PersonInit {
            age_months: ADULT_AGE,
            parent: self.ppl.parent[i],
            home: NO_HOME,
            skill,
            appetite: rng.uniform(1.8, 3.0),
            shelter_need: rng.uniform(0.8, 1.2),
            status: rng.uniform(0.05, 0.3),
            patience,
            price_sens: rng.uniform(10.0, 30.0),
            leisure,
            risk,
            search: rng.f64(),
            family: blend(rng.uniform(0.5, 1.5), p_family, has_parent),
            reservation_wage: (avg_wage * skill * leisure * 0.8) as i64,
            pref,
        };
        self.ppl.write(i, init);
        self.ppl.hc[i] = hc;
        // a home of their own
        let ledger = &mut self.ledger;
        let h = self.homes.spawn(known, || ledger.open());
        self.homes.members[h].push(i as u32);
        self.ppl.home[i] = h as u32;
        self.homes.refresh(h, &self.ppl);
        self.homes.minors[old_home] = self.homes.minors[old_home].saturating_sub(1);
        let near = self.homes.tile[old_home];
        self.house(h, near);
        // gift: a tenth of the old home's cash if it is above its buffer
        if self.homes.has_adults(old_home) {
            let cash = self.ledger.balance(self.homes.account[old_home]);
            let reserve = (self.homes.patience[old_home] * DAYS_PER_MONTH as f64 * self.necessities_daily(old_home)) as i64;
            if cash > reserve {
                let gift = cash / 10;
                if gift > 0 {
                    self.ledger.transfer(self.homes.account[old_home], self.homes.account[h], gift, self.day);
                    self.homes.income_month[h] += gift;
                }
            }
        }
        if has_parent {
            self.mobility.push((p_skill, skill));
        }
        self.month_hc_sum += hc;
        if self.cfg.watch_home == Some(old_home) || self.cfg.watch_person == Some(i) {
            self.watch.log(self.day, "adult", &format!("p{} leaves home{} for home{}: hc {:.2}, skill {:.2} (parent {:.2}), pref {:.2}", i, old_home, h, hc, skill, p_skill, pref));
        }
    }

    fn retire(&mut self, i: usize) {
        if self.ppl.is_employed(i) {
            self.remove_from_firm(i);
        }
        self.ppl.retired[i] = true;
    }

    pub(crate) fn remove_from_firm(&mut self, i: usize) {
        let f = self.ppl.employer[i] as usize;
        if let Some(pos) = self.firms.employees[f].iter().position(|&x| x as usize == i) {
            self.firms.employees[f].swap_remove(pos);
            self.firms.effective_labor[f] -= self.labor_of(i, f);
        }
        self.ppl.employer[i] = crate::people::NO_FIRM;
    }

    /// Move everything a home has into another and dissolve it.
    fn merge_homes(&mut self, from: usize, into: usize) {
        let cash = self.ledger.balance(self.homes.account[from]);
        if cash > 0 {
            self.ledger.transfer(self.homes.account[from], self.homes.account[into], cash, self.day);
        }
        self.homes.bonds[into] += self.homes.bonds[from];
        self.homes.bonds[from] = 0;
        let holdings = std::mem::take(&mut self.homes.holdings[from]);
        for &(f, sh) in &holdings {
            let removed = self.firms.remove_holder(f as usize, from as u32, sh);
            debug_assert_eq!(removed, sh);
            self.firms.add_holder(f as usize, into as u32, sh);
            self.homes.add_shares(into, f, sh);
        }
        self.homes.holdings[from] = holdings;
        self.homes.holdings[from].clear();
        let members = std::mem::take(&mut self.homes.members[from]);
        for &m in &members {
            self.ppl.home[m as usize] = into as u32;
            self.homes.members[into].push(m);
        }
        self.homes.members[from] = members;
        self.homes.members[from].clear();
        for c in 0..self.ppl.n {
            if self.ppl.alive[c] && self.ppl.home[c] as usize == from {
                self.ppl.home[c] = into as u32;
            }
        }
        self.homes.minors[into] += self.homes.minors[from];
        self.homes.minors[from] = 0;
        for &f in &self.firms.active_list {
            if self.firms.owner[f as usize] as usize == from {
                self.firms.owner[f as usize] = into as u32;
            }
        }
        self.homes.firms_owned[into] += self.homes.firms_owned[from];
        self.homes.income_ema[into] += self.homes.income_ema[from];
        self.homes.income_month[into] += self.homes.income_month[from];
        self.unhouse(from);
        self.homes.deactivate(from);
        self.homes.refresh(into, &self.ppl);
        if self.homes.tile[into] == crate::city::NO_TILE {
            self.house(into, crate::city::NO_TILE);
        }
    }

    /// Single adults aged 20-60 meet; like pairs with like (age, skill, politics).
    pub(crate) fn unions(&mut self) -> u32 {
        self.job_seekers.clear();
        for i in 0..self.ppl.n {
            if !self.ppl.is_adult(i) {
                continue;
            }
            let age = self.ppl.age_months[i];
            let h = self.ppl.home[i] as usize;
            if age >= 20 * 12 && age <= 60 * 12 && self.homes.members[h].len() == 1 && self.rng_demo.chance(0.06) {
                self.job_seekers.push(i as u32);
            }
        }
        self.rng_demo.shuffle(&mut self.job_seekers);
        let mut formed = 0;
        let mut k = 0;
        while k + 1 < self.job_seekers.len() {
            let a = self.job_seekers[k] as usize;
            let b = self.job_seekers[k + 1] as usize;
            k += 2;
            let d_age = (self.ppl.age_months[a] as f64 - self.ppl.age_months[b] as f64).abs() / 120.0;
            let d_pref = (self.ppl.pref[a] - self.ppl.pref[b]).abs();
            let d_skill = (self.ppl.skill[a] - self.ppl.skill[b]).abs();
            let like = (-d_age - 2.0 * d_pref - 2.0 * d_skill).exp();
            if !self.rng_demo.chance(like) {
                continue;
            }
            let ha = self.ppl.home[a] as usize;
            let hb = self.ppl.home[b] as usize;
            if ha == hb {
                continue;
            }
            // the poorer moves in with the richer
            let (from, into) = if self.ledger.balance(self.homes.account[ha]) >= self.ledger.balance(self.homes.account[hb]) { (hb, ha) } else { (ha, hb) };
            self.merge_homes(from, into);
            formed += 1;
            if self.cfg.watch_home == Some(into) || self.cfg.watch_person == Some(a) || self.cfg.watch_person == Some(b) {
                self.watch.log(self.day, "union", &format!("p{} and p{} form home{}", a, b, into));
            }
        }
        formed
    }

    /// Two-adult homes split at ~1.2% a year: the leaver takes half of everything, minors stay.
    pub(crate) fn separations(&mut self) -> u32 {
        let mut count = 0;
        for h in 0..self.homes.n {
            if !self.homes.active[h] || self.homes.members[h].len() != 2 || !self.rng_demo.chance(0.001) {
                continue;
            }
            let leaver = self.homes.members[h][self.rng_demo.below(2)] as usize;
            let known = self.homes.known[h];
            let ledger = &mut self.ledger;
            let nh = self.homes.spawn(known, || ledger.open());
            let pos = self.homes.members[h].iter().position(|&m| m as usize == leaver).unwrap();
            self.homes.members[h].swap_remove(pos);
            self.homes.members[nh].push(leaver as u32);
            self.ppl.home[leaver] = nh as u32;
            let cash = self.ledger.balance(self.homes.account[h]) / 2;
            if cash > 0 {
                self.ledger.transfer(self.homes.account[h], self.homes.account[nh], cash, self.day);
            }
            let b = self.homes.bonds[h] / 2;
            self.homes.bonds[h] -= b;
            self.homes.bonds[nh] += b;
            let holdings = self.homes.holdings[h].clone();
            for (f, sh) in holdings {
                let q = sh / 2;
                if q > 0 {
                    self.transfer_shares(f as usize, h, nh, q);
                }
            }
            self.homes.income_ema[nh] = self.homes.income_ema[h] / 2.0;
            self.homes.income_ema[h] /= 2.0;
            self.homes.refresh(h, &self.ppl);
            self.homes.refresh(nh, &self.ppl);
            let near = self.homes.tile[h];
            self.house(nh, near);
            count += 1;
            if self.cfg.watch_home == Some(h) || self.cfg.watch_person == Some(leaver) {
                self.watch.log(self.day, "separation", &format!("p{} leaves home{} for home{}", leaver, h, nh));
            }
        }
        count
    }

    /// Death. The home keeps everything while another adult lives there; when
    /// the last adult dies the estate goes to the homes of living adult children
    /// (equal parts, cash after inheritance tax), else to a random home.
    fn die(&mut self, i: usize) {
        if self.ppl.is_employed(i) {
            self.remove_from_firm(i);
        }
        let h = self.ppl.home[i] as usize;
        let was_adult = self.ppl.age_months[i] >= ADULT_AGE;
        self.deaths_age_sum += self.ppl.age_months[i] as f64 / 12.0;
        self.deaths_total += 1;
        if !was_adult {
            self.homes.minors[h] = self.homes.minors[h].saturating_sub(1);
            self.ppl.bury(i);
            return;
        }
        if let Some(pos) = self.homes.members[h].iter().position(|&m| m as usize == i) {
            self.homes.members[h].swap_remove(pos);
        }
        if !self.homes.members[h].is_empty() {
            self.homes.refresh(h, &self.ppl);
            self.ppl.bury(i);
            return;
        }
        // last adult: settle the estate
        let n = self.ppl.n;
        let mut heirs: Vec<usize> = Vec::new();
        for c in 0..n {
            if c != i && self.ppl.is_adult(c) && self.ppl.parent[c] as usize == i {
                let hc = self.ppl.home[c] as usize;
                if !heirs.contains(&hc) {
                    heirs.push(hc);
                }
            }
        }
        if heirs.is_empty() {
            let homes: Vec<usize> = (0..self.homes.n).filter(|&x| x != h && self.homes.has_adults(x)).collect();
            assert!(!homes.is_empty(), "day {}: population extinct", self.day);
            heirs.push(homes[self.rng_demo.below(homes.len())]);
        }
        let acc = self.homes.account[h];
        let cash = self.ledger.balance(acc);
        let tax = (cash as f64 * self.gov.policy.inheritance_tax).floor() as i64;
        if tax > 0 {
            self.ledger.transfer(acc, self.gov.account, tax, self.day);
            self.gov.inheritance_tax_month += tax;
        }
        let estate = cash - tax;
        let k = heirs.len() as i64;
        for (j, &hh) in heirs.iter().enumerate() {
            let mut part = estate / k;
            if j == 0 {
                part += estate - (estate / k) * k;
            }
            if part > 0 {
                self.ledger.transfer(acc, self.homes.account[hh], part, self.day);
                self.homes.income_month[hh] += part;
            }
            let b = self.homes.bonds[h] / k + if j == 0 { self.homes.bonds[h] % k } else { 0 };
            self.homes.bonds[hh] += b;
        }
        self.homes.bonds[h] = 0;
        let holdings = std::mem::take(&mut self.homes.holdings[h]);
        for &(f, sh) in &holdings {
            let each = sh / heirs.len() as u32;
            let rem = sh % heirs.len() as u32;
            let removed = self.firms.remove_holder(f as usize, h as u32, sh);
            debug_assert_eq!(removed, sh);
            for (j, &hh) in heirs.iter().enumerate() {
                let q = each + if j == 0 { rem } else { 0 };
                if q > 0 {
                    self.firms.add_holder(f as usize, hh as u32, q);
                    self.homes.add_shares(hh, f, q);
                }
            }
        }
        self.homes.holdings[h] = holdings;
        self.homes.holdings[h].clear();
        let guardian = heirs[0];
        for &f in &self.firms.active_list {
            if self.firms.owner[f as usize] as usize == h {
                self.firms.owner[f as usize] = guardian as u32;
            }
        }
        for c in 0..n {
            if self.ppl.alive[c] && c != i && self.ppl.home[c] as usize == h {
                self.ppl.home[c] = guardian as u32;
                self.homes.minors[guardian] += 1;
            }
        }
        self.homes.minors[h] = 0;
        debug_assert_eq!(self.ledger.balance(acc), 0);
        if self.cfg.watch_home == Some(h) || self.cfg.watch_person == Some(i) {
            self.watch.log(self.day, "death", &format!("p{} dies at {}; home{} dissolved, estate {} cents to homes {:?}", i, self.ppl.age_months[i] / 12, h, cash, heirs));
        }
        self.unhouse(h);
        self.homes.deactivate(h);
        self.ppl.bury(i);
    }

    /// Newcomers arrive when the city has room and work: up to a tenth of the
    /// spare housing a month, one adult per home, aged 20-35, with no cash.
    /// Returns how many arrived.
    pub(crate) fn immigration(&mut self, unemployment: f64) -> u32 {
        let room = self.city.capacity_of(crate::city::Zone::Residential) as i64 - self.city.occupants_of(crate::city::Zone::Residential) as i64;
        if room <= 0 || unemployment > 0.08 {
            return 0;
        }
        let n = ((room / 10) as u32).clamp(1, 12);
        let avg_wage = self.stats.last_wage;
        let mut arrived = 0;
        for _ in 0..n {
            let rng = &mut self.rng_demo;
            let skill = 0.5 + (rng.f64() + rng.f64()) * 0.5;
            let patience = rng.uniform(0.5, 3.0);
            let risk = rng.f64();
            let leisure = rng.uniform(0.8, 1.2);
            let pref = (crate::politics::prior(patience, risk, skill) + rng.uniform(-0.15, 0.15)).clamp(0.0, 1.0);
            let init = PersonInit {
                age_months: rng.below(16 * 12) as u32 + 20 * 12,
                parent: NO_PARENT,
                home: NO_HOME,
                skill,
                appetite: rng.uniform(1.8, 3.0),
                shelter_need: rng.uniform(0.8, 1.2),
                status: rng.uniform(0.05, 0.3),
                patience,
                price_sens: rng.uniform(10.0, 30.0),
                leisure,
                risk,
                search: rng.f64(),
                family: rng.uniform(0.5, 1.5),
                reservation_wage: (avg_wage * skill * leisure * 0.7) as i64,
                pref,
            };
            let i = self.ppl.spawn(init);
            // shopping network copied from a random established home
            let known = {
                let homes: Vec<usize> = (0..self.homes.n).filter(|&h| self.homes.has_adults(h)).collect();
                if homes.is_empty() { [[crate::homes::NO_FIRM; crate::homes::K]; crate::goods::NG] } else { self.homes.known[homes[self.rng_demo.below(homes.len())]] }
            };
            let ledger = &mut self.ledger;
            let h = self.homes.spawn(known, || ledger.open());
            self.homes.members[h].push(i as u32);
            self.ppl.home[i] = h as u32;
            self.homes.refresh(h, &self.ppl);
            self.house(h, crate::city::NO_TILE);
            arrived += 1;
        }
        if arrived > 0 {
            self.events.log(self.day, "city", &format!("{} newcomers moved into the city", arrived));
        }
        arrived
    }

    /// Correlation of skill between parents and children who came of age so far.
    pub(crate) fn mobility_correlation(&self) -> f64 {
        let n = self.mobility.len() as f64;
        if n < 3.0 {
            return 0.0;
        }
        let (mut sx, mut sy, mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for &(x, y) in &self.mobility {
            sx += x;
            sy += y;
            sxx += x * x;
            syy += y * y;
            sxy += x * y;
        }
        let cov = sxy / n - (sx / n) * (sy / n);
        let vx = sxx / n - (sx / n).powi(2);
        let vy = syy / n - (sy / n).powi(2);
        if vx <= 0.0 || vy <= 0.0 { 0.0 } else { cov / (vx * vy).sqrt() }
    }
}
