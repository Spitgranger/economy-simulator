//! Schooling. Children aged 6-17 are enrolled every month: privately, if a
//! known school has a seat, teaches better than the public system and costs
//! under 15% of the home's income; otherwise in the public school. Quality
//! is pupils per teacher and teacher skill; a child's human capital grows by
//! quality/144 a month, so twelve years of full-quality schooling reach 1.0.

use crate::goods::{EDUCATION, PRIVATE_CLASS_SIZE};
use crate::homes::NO_FIRM as NO_KNOWN;
use crate::sim::{World, DAYS_PER_MONTH};

pub const MONTHS_OF_SCHOOL: f64 = 144.0;

impl World {
    /// Teaching quality of a staff of `teachers` (with summed skill) for `pupils`.
    fn quality(teachers: usize, teacher_skill_sum: f64, pupils: u32, target_ratio: f64) -> f64 {
        if teachers == 0 || pupils == 0 {
            return if teachers == 0 { 0.0 } else { 1.0 };
        }
        let ratio = pupils as f64 / teachers as f64;
        let size = (target_ratio / ratio).min(1.0);
        let skill = (teacher_skill_sum / teachers as f64 / 1.2).clamp(0.6, 1.0);
        size * skill
    }

    fn staff_skill(&self, f: usize) -> f64 {
        self.firms.employees[f].iter().map(|&t| self.ppl.skill[t as usize]).sum()
    }

    /// Monthly: quality from last month's rosters, then enrollment and learning.
    pub(crate) fn enroll_and_teach(&mut self) {
        // 1. quality of every school from last month's pupils and this month's staff
        let public_q = match self.public_school {
            Some(s) if self.gov.policy.class_size > 0.0 => {
                let q = Self::quality(self.firms.employees[s].len(), self.staff_skill(s), self.public_pupils, 20.0);
                // overcrowded buildings: quality capped by school room in the city
                let room = self.city.capacity_of(crate::city::Zone::School) as f64;
                q * (room / self.public_pupils.max(1) as f64).min(1.0)
            }
            _ => 0.0,
        };
        self.public_quality = public_q;
        for &f in &self.firms.active_list {
            let f = f as usize;
            if !self.firms.is_school(f) || self.firms.public[f] {
                continue;
            }
            let q = Self::quality(self.firms.employees[f].len(), self.staff_skill(f), self.firms.pupils[f], PRIVATE_CLASS_SIZE as f64);
            self.firms.quality[f] = q;
            // seats on offer this month
            self.firms.inventory[f] = self.firms.employees[f].len() as i64 * PRIVATE_CLASS_SIZE as i64;
            self.firms.pupils[f] = 0;
        }
        if let Some(s) = self.public_school {
            self.firms.quality[s] = public_q;
        }

        // 2. enrollment
        let mut public_pupils = 0u32;
        let mut private_pupils = 0u32;
        let n = self.ppl.n;
        for c in 0..n {
            if !self.ppl.is_pupil(c) {
                continue;
            }
            let h = self.ppl.home[c] as usize;
            let mut q_got = public_q;
            let mut private = false;
            if self.homes.has_adults(h) {
                let budget = (0.15 * self.homes.income_ema[h] * DAYS_PER_MONTH as f64) as i64;
                let cash = self.ledger.balance(self.homes.account[h]);
                let mut best: Option<(usize, f64)> = None;
                // stay where enrolled while the school is still acceptable
                let cur = self.ppl.school[c];
                if cur != crate::people::NO_FIRM {
                    let f = cur as usize;
                    if self.firms.active[f] && !self.firms.public[f] && self.firms.inventory[f] > 0
                        && self.firms.quality[f] > public_q && self.firms.price[f] <= budget && self.firms.price[f] <= cash
                    {
                        best = Some((f, self.firms.quality[f]));
                    }
                }
                for &f in &self.homes.known[h][EDUCATION] {
                    if best.is_some() {
                        break;
                    }
                    if f == NO_KNOWN {
                        continue;
                    }
                    let f = f as usize;
                    if !self.firms.active[f] || self.firms.public[f] {
                        continue;
                    }
                    let tuition = self.firms.price[f];
                    let q = self.firms.quality[f];
                    if q > public_q + 0.05 && tuition <= budget && tuition <= cash && best.map_or(true, |(_, bq)| q > bq) {
                        best = Some((f, q));
                    }
                }
                if let Some((f, q)) = best {
                    if self.firms.inventory[f] > 0 {
                        let tuition = self.firms.price[f];
                        self.ledger.transfer(self.homes.account[h], self.firms.account[f], tuition, self.day);
                        self.homes.tuition_month[h] += tuition;
                        self.month_tuition += tuition;
                        self.ppl.school[c] = f as u32;
                        self.firms.inventory[f] -= 1;
                        self.firms.sales_month[f] += 1;
                        self.firms.revenue_month[f] += tuition;
                        self.firms.pupils[f] += 1;
                        q_got = q;
                        private = true;
                    } else {
                        self.firms.lost_sales_month[f] += 1;
                    }
                }
            }
            if !private {
                public_pupils += 1;
                self.ppl.school[c] = crate::people::NO_FIRM;
            } else {
                private_pupils += 1;
            }
            self.ppl.private_school[c] = private;
            self.ppl.hc[c] = (self.ppl.hc[c] + q_got / MONTHS_OF_SCHOOL).min(1.0);
        }
        self.public_pupils = public_pupils;
        self.private_pupils = private_pupils;
        if let Some(s) = self.public_school {
            self.firms.pupils[s] = public_pupils;
            self.firms.sales_month[s] = public_pupils as i64;
        }
    }

    /// The public school: staff to the policy class size, pay at the policy
    /// multiple of the average wage, hand cash above two weeks of payroll back.
    pub(crate) fn public_school_decisions(&mut self, s: usize) {
        let class = self.gov.policy.class_size;
        self.firms.target_workers[s] = if class > 0.0 { (self.public_pupils as f64 / class).ceil() as u32 } else { 0 };
        self.firms.wage_rate[s] = ((self.gov.policy.teacher_pay * self.stats.last_wage).round() as i64).max(1);
        let n = self.firms.employees[s].len() as i64;
        let two_weeks = 2 * n * self.firms.wage_rate[s];
        let cash = self.ledger.balance(self.firms.account[s]);
        if cash > two_weeks {
            self.ledger.transfer(self.firms.account[s], self.gov.account, cash - two_weeks, self.day);
        }
        if self.cfg.watch_firm == Some(s) {
            self.watch.log(
                self.day,
                "school",
                &format!("public school: pupils={} teachers={} target={} quality={:.2} pay={}", self.public_pupils, n, self.firms.target_workers[s], self.public_quality, self.firms.wage_rate[s]),
            );
        }
        self.firms.age_months[s] += 1;
        self.firms.lost_sales_month[s] = 0;
        self.firms.revenue_month[s] = 0;
        self.firms.wages_paid_month[s] = 0;
    }
}
