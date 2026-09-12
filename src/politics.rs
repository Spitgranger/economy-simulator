//! Political preferences and elections. A person's preference is a point on a
//! redistribution axis, anchored by personality and moved by experience.
//! Parties are fixed points on that axis; a platform maps to a `Policy` by one
//! formula, so an election is just another way to set the government's levers.

use crate::government::Policy;
use crate::sim::World;

pub const N_PARTIES: usize = 3;
pub const PARTIES: [(&str, f64); N_PARTIES] = [("Market", 0.15), ("Centre", 0.5), ("Labour", 0.85)];

/// Policy bundle at position p on the axis; the minimum wage is set relative
/// to the average pay at election time.
pub fn platform(p: f64, avg_pay: f64) -> Policy {
    let mut pol = Policy::default();
    pol.income_tax = 0.35 * p;
    pol.dividend_tax = 0.4 * p;
    pol.sales_tax = [0.02 * p, 0.02 * p, 0.5 * p, 0.0];
    pol.benefit_rate = 0.6 * p;
    pol.basic_income = 0.1 * p;
    pol.pension = 0.4 * p;
    pol.child_benefit = 0.1 * p;
    pol.inheritance_tax = 0.4 * p;
    pol.class_size = 40.0 - 25.0 * p;
    pol.teacher_pay = 0.9 + 0.3 * p;
    pol.ground_rent = 0.005 + 0.01 * p;
    pol.min_wage = (0.8 * p * avg_pay).round() as i64;
    pol.surplus_dividend = 1.0;
    pol
}

/// Personality anchor: patient and low-skill lean redistribution, risk-takers lean market.
pub fn prior(patience: f64, risk: f64, skill: f64) -> f64 {
    (0.5 + 0.2 * (patience - 1.75) / 1.25 - 0.4 * (risk - 0.5) - 0.3 * (skill - 1.0)).clamp(0.0, 1.0)
}

impl World {
    /// Monthly drift of a person's preference toward what this month suggested,
    /// judged on their own job and their home's money. Called before resets.
    pub(crate) fn update_preference(&mut self, i: usize) {
        let h = self.ppl.home[i] as usize;
        let mut target = self.ppl.pref_prior[i];
        if !self.ppl.is_employed(i) && !self.ppl.retired[i] {
            target += 0.35;
        }
        if self.homes.tile[h] == crate::city::NO_TILE {
            target += 0.2;
        }
        if self.homes.transfers_month[h] > self.homes.tax_month[h] {
            target += 0.15;
        } else if self.homes.tax_month[h] > 0 {
            target -= 0.1;
        }
        let income = self.homes.income_month[h].max(1) as f64;
        if (self.homes.dividend_month[h] + self.homes.interest_month[h]) as f64 > 0.5 * income {
            target -= 0.3;
        }
        let wealth = self.ledger.balance(self.homes.account[h]) + self.homes.assets_value[h];
        if wealth as f64 > 6.0 * crate::sim::DAYS_PER_MONTH as f64 * self.necessities_daily(h) {
            target -= 0.2;
        }
        let target = target.clamp(0.0, 1.0);
        self.ppl.pref[i] += 0.1 * (target - self.ppl.pref[i]);
    }

    /// Expected vote shares if an election were held now: the three parties
    /// plus, in interactive mode, the player's bundle as "Government".
    /// Returns (shares over N_PARTIES + 1, mean preference). Does not change state.
    pub fn poll(&self) -> ([f64; N_PARTIES + 1], f64) {
        let cpi = self.stats.last_cpi.max(1e-9);
        let player_pos = self.gov.player_policy.as_ref().map(|p| p.position());
        let incumbent = self.gov.incumbent;
        let mut shares = [0f64; N_PARTIES + 1];
        let mut pref_sum = 0.0;
        let mut voters = 0usize;
        for i in 0..self.ppl.n {
            if !self.ppl.is_adult(i) {
                continue;
            }
            voters += 1;
            let x = self.ppl.pref[i];
            pref_sum += x;
            let h = self.ppl.home[i] as usize;
            let real_now = self.homes.income_ema[h] / cpi / self.homes.members[h].len().max(1) as f64;
            let real_then = self.ppl.real_income_at_election[i];
            let delta = if real_then > 0.0 { (real_now / real_then - 1.0).clamp(-0.5, 0.5) } else { 0.0 };
            let mut w = [0f64; N_PARTIES + 1];
            for k in 0..N_PARTIES {
                let mut u = -(x - PARTIES[k].1).abs() / 0.2;
                if Some(k) == incumbent && !self.gov.player_in_power {
                    u += 3.0 * delta;
                }
                w[k] = u.exp();
            }
            if let Some(pp) = player_pos {
                let mut u = -(x - pp).abs() / 0.2;
                if self.gov.player_in_power {
                    u += 3.0 * delta;
                }
                w[N_PARTIES] = u.exp();
            }
            let total: f64 = w.iter().sum();
            for k in 0..=N_PARTIES {
                shares[k] += w[k] / total;
            }
        }
        let n = voters.max(1) as f64;
        for k in 0..=N_PARTIES {
            shares[k] /= n;
        }
        (shares, pref_sum / n)
    }

    /// Plurality election with softmax voting over platform distance and a
    /// retrospective term on the incumbent: the voter's home real income over the
    /// term. In interactive mode the player's bundle stands as "Government".
    pub(crate) fn election(&mut self, month: u32) {
        let cpi = self.stats.last_cpi.max(1e-9);
        let n_opts = if self.gov.player_policy.is_some() { N_PARTIES + 1 } else { N_PARTIES };
        let player_pos = self.gov.player_policy.as_ref().map(|p| p.position()).unwrap_or(0.5);
        let mut votes = [0usize; N_PARTIES + 1];
        let incumbent = self.gov.incumbent;
        let mut voters = 0usize;
        for i in 0..self.ppl.n {
            if !self.ppl.is_adult(i) {
                continue;
            }
            voters += 1;
            let x = self.ppl.pref[i];
            let h = self.ppl.home[i] as usize;
            let real_now = self.homes.income_ema[h] / cpi / self.homes.members[h].len().max(1) as f64;
            let real_then = self.ppl.real_income_at_election[i];
            let delta = if real_then > 0.0 { (real_now / real_then - 1.0).clamp(-0.5, 0.5) } else { 0.0 };
            let mut w = [0f64; N_PARTIES + 1];
            for k in 0..n_opts {
                let pos = if k < N_PARTIES { PARTIES[k].1 } else { player_pos };
                let mut u = -(x - pos).abs() / 0.2;
                let is_incumbent = if k < N_PARTIES { Some(k) == incumbent && !self.gov.player_in_power } else { self.gov.player_in_power };
                if is_incumbent {
                    u += 3.0 * delta;
                }
                w[k] = u.exp();
            }
            let total: f64 = w[..n_opts].iter().sum();
            let mut r = self.rng_politics.f64() * total;
            let mut k = 0;
            while k + 1 < n_opts {
                r -= w[k];
                if r < 0.0 {
                    break;
                }
                k += 1;
            }
            votes[k] += 1;
            self.ppl.real_income_at_election[i] = real_now;
        }
        let n = voters.max(1) as f64;
        for k in 0..N_PARTIES {
            self.gov.vote_shares[k] = votes[k] as f64 / n;
        }
        self.gov.player_share = votes[N_PARTIES] as f64 / n;
        let winner = (0..n_opts).max_by_key(|&k| votes[k]).unwrap();
        let mean_pref: f64 = (0..self.ppl.n).filter(|&i| self.ppl.is_adult(i)).map(|i| self.ppl.pref[i]).sum::<f64>() / n;
        self.gov.elections_held += 1;
        let mut shares: Vec<String> = (0..N_PARTIES).map(|k| format!("{} {:.1}%", PARTIES[k].0, self.gov.vote_shares[k] * 100.0)).collect();
        if n_opts > N_PARTIES {
            shares.push(format!("Government {:.1}%", self.gov.player_share * 100.0));
        }
        if winner == N_PARTIES {
            // the player keeps (or regains) control
            self.gov.player_in_power = true;
            self.gov.incumbent = None;
            self.gov.locked_until = 0;
            if let Some(p) = self.gov.player_policy.clone() {
                self.gov.policy = p;
            }
            self.events.log(self.day, "election", &format!("month {}: the Government is re-elected ({}); mean preference {:.2}", month, shares.join(", "), mean_pref));
        } else {
            self.gov.incumbent = Some(winner);
            self.gov.policy = platform(PARTIES[winner].1, self.stats.last_wage);
            if self.gov.player_policy.is_some() {
                self.gov.player_in_power = false;
                self.gov.locked_until = month + self.cfg.election_every_years * crate::sim::MONTHS_PER_YEAR;
            }
            self.events.log(
                self.day,
                "election",
                &format!("month {}: {} wins ({}); mean preference {:.2}; policy -> {}", month, PARTIES[winner].0, shares.join(", "), mean_pref, self.gov.policy.describe()),
            );
        }
    }
}
