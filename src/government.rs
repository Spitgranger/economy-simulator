//! Government: a ledger account plus a set of levers. Every policy routes
//! through a mechanism agents already use - it changes a price (sales tax),
//! a payoff (income and dividend tax, benefits, the universal dividend) or a
//! constraint (minimum wage). Policies can be rescheduled at any month, which
//! is the hook elections will use.

use crate::goods::{LUXURY, NG};
use crate::ledger::Account;

#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    pub income_tax: f64,
    pub sales_tax: [f64; NG],
    pub dividend_tax: f64,
    /// weekly unemployment benefit as a fraction of the average gross wage
    pub benefit_rate: f64,
    /// weekly payment to every household as a fraction of the average gross wage,
    /// paid whether or not the treasury can cover it (the gap is borrowed)
    pub basic_income: f64,
    /// weekly pension per retiree as a fraction of the average gross wage
    pub pension: f64,
    /// weekly payment per minor child to the parent, fraction of the average wage;
    /// also the proxy for public education quality
    pub child_benefit: f64,
    /// share of a cash estate taken at death
    pub inheritance_tax: f64,
    /// public school target pupils per teacher; 0 = no public schooling
    pub class_size: f64,
    /// public teacher pay as a multiple of the average wage
    pub teacher_pay: f64,
    /// floor on weekly gross pay, cents; 0 = none
    pub min_wage: i64,
    /// share of treasury above one month of benefits returned equally to all
    /// households each month; 1.0 = balanced budget over time
    pub surplus_dividend: f64,
    /// central bank policy rate set by hand (annual); negative = Taylor rule
    pub policy_rate: f64,
    /// new money minted into the treasury each month, as a share of the money supply
    pub print_rate: f64,
    /// monthly ground rent as a share of land value, paid by homes (x1) and firms (x2)
    pub ground_rent: f64,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            income_tax: 0.0,
            sales_tax: [0.0; NG],
            dividend_tax: 0.0,
            benefit_rate: 0.0,
            basic_income: 0.0,
            pension: 0.0,
            child_benefit: 0.0,
            inheritance_tax: 0.0,
            class_size: 0.0,
            teacher_pay: 1.0,
            min_wage: 0,
            surplus_dividend: 1.0,
            policy_rate: -1.0,
            print_rate: 0.0,
            ground_rent: 0.01,
        }
    }
}

impl Policy {
    /// Apply `key=value`. Returns Err on an unknown key or bad value.
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let num = |v: &str| v.parse::<f64>().map_err(|_| format!("bad number '{}' for {}", v, key));
        match key {
            "income-tax" => self.income_tax = num(value)?,
            "sales-tax" => {
                let v = num(value)?;
                for g in 0..NG {
                    if g != LUXURY || self.sales_tax[LUXURY] <= self.sales_tax[0] {
                        self.sales_tax[g] = v;
                    }
                }
            }
            "luxury-tax" => self.sales_tax[LUXURY] = num(value)?,
            "dividend-tax" => self.dividend_tax = num(value)?,
            "benefit" => self.benefit_rate = num(value)?,
            "basic-income" => self.basic_income = num(value)?,
            "pension" => self.pension = num(value)?,
            "child-benefit" => self.child_benefit = num(value)?,
            "inheritance-tax" => self.inheritance_tax = num(value)?,
            "class-size" => self.class_size = num(value)?,
            "teacher-pay" => self.teacher_pay = num(value)?,
            "min-wage" => self.min_wage = num(value)? as i64,
            "surplus-dividend" => self.surplus_dividend = num(value)?,
            "policy-rate" => self.policy_rate = if value == "auto" { -1.0 } else { num(value)? },
            "print-rate" => self.print_rate = num(value)?,
            "ground-rent" => self.ground_rent = num(value)?,
            _ => return Err(format!("unknown policy '{}'", key)),
        }
        Ok(())
    }

    /// Where this bundle sits on the redistribution axis: the average of the
    /// levers' positions relative to the party formula in `politics::platform`.
    pub fn position(&self) -> f64 {
        let parts = [
            self.income_tax / 0.35,
            self.dividend_tax / 0.4,
            self.benefit_rate / 0.6,
            self.pension / 0.4,
            self.inheritance_tax / 0.4,
            ((40.0 - self.class_size) / 25.0).clamp(0.0, 1.6),
        ];
        (parts.iter().sum::<f64>() / parts.len() as f64).clamp(0.0, 1.0)
    }

    pub fn describe(&self) -> String {
        format!(
            "income tax {:.0}%, sales tax {:.0}% (luxury {:.0}%), dividend tax {:.0}%, inheritance tax {:.0}%, benefit {:.0}% of avg wage, basic income {:.0}%, pension {:.0}%, child benefit {:.0}%, min wage {}, public school class size {:.0} at {:.2}x pay, surplus returned {:.0}%, policy rate {}, printing {:.1}%/month",
            self.income_tax * 100.0, self.sales_tax[0] * 100.0, self.sales_tax[LUXURY] * 100.0,
            self.dividend_tax * 100.0, self.inheritance_tax * 100.0, self.benefit_rate * 100.0, self.basic_income * 100.0,
            self.pension * 100.0, self.child_benefit * 100.0, self.min_wage, self.class_size, self.teacher_pay, self.surplus_dividend * 100.0,
            if self.policy_rate < 0.0 { "auto".to_string() } else { format!("{:.1}%", self.policy_rate * 100.0) }, self.print_rate * 100.0
        )
    }
}

pub struct Government {
    pub account: Account,
    pub policy: Policy,
    /// (month at which it takes effect, key, value)
    pub schedule: Vec<(u32, String, String)>,

    // monthly counters
    pub income_tax_month: i64,
    pub sales_tax_month: i64,
    pub dividend_tax_month: i64,
    pub benefits_month: i64,
    pub dividend_paid_month: i64,
    pub benefit_last_week: i64,
    pub basic_income_last_week: i64,
    pub pension_last_week: i64,
    pub child_benefit_last_week: i64,
    pub education_last_week: i64,
    pub education_month: i64,
    pub pending_print: i64, // one-off print (+) or burn (-) requested by the player, cents
    pub printed_month: i64,
    pub pending_builds: Vec<(usize, usize, u8)>, // (x, y, zone) ordered by the player
    pub rent_month: i64,
    pub inheritance_tax_month: i64,
    pub pensions_month: i64,
    pub child_benefit_month: i64,
    pub unfunded_benefit_weeks: u32,
    pub bailouts_month: i64,

    // debt: 12-month bonds, all holders share one pro-rata pool
    pub debt: i64,
    pub bond_rate: f64,
    pub issues: Vec<(u32, i64)>, // (maturity month, face)
    pub bond_interest_month: i64,
    pub issued_month: i64,
    pub issued_to_bank_month: i64,
    pub redeemed_month: i64,
    pub debt_brake: bool,

    // politics
    pub incumbent: Option<usize>,
    pub vote_shares: [f64; crate::politics::N_PARTIES],
    pub elections_held: u32,

    // interactive mode: the player's bundle stands at elections as "Government"
    pub player_policy: Option<Policy>,
    pub player_in_power: bool,
    pub locked_until: u32, // month until which a winning party governs instead
    pub player_share: f64, // player's share at the last election or poll
}

impl Government {
    pub fn new(account: Account, policy: Policy) -> Government {
        Government {
            account,
            policy,
            schedule: Vec::new(),
            income_tax_month: 0,
            sales_tax_month: 0,
            dividend_tax_month: 0,
            benefits_month: 0,
            dividend_paid_month: 0,
            benefit_last_week: 0,
            basic_income_last_week: 0,
            pension_last_week: 0,
            child_benefit_last_week: 0,
            education_last_week: 0,
            education_month: 0,
            pending_print: 0,
            printed_month: 0,
            pending_builds: Vec::new(),
            rent_month: 0,
            inheritance_tax_month: 0,
            pensions_month: 0,
            child_benefit_month: 0,
            unfunded_benefit_weeks: 0,
            bailouts_month: 0,
            debt: 0,
            bond_rate: 0.03,
            issues: Vec::new(),
            bond_interest_month: 0,
            issued_month: 0,
            issued_to_bank_month: 0,
            redeemed_month: 0,
            debt_brake: false,
            incumbent: None,
            vote_shares: [0.0; crate::politics::N_PARTIES],
            elections_held: 0,
            player_policy: None,
            player_in_power: false,
            locked_until: 0,
            player_share: 0.0,
        }
    }

    pub fn tax_month(&self) -> i64 {
        self.income_tax_month + self.sales_tax_month + self.dividend_tax_month + self.inheritance_tax_month + self.rent_month
    }

    /// All weekly transfer programs, last week's total: the base for next month's projection.
    pub fn transfers_last_week(&self) -> i64 {
        self.benefit_last_week + self.basic_income_last_week + self.pension_last_week + self.child_benefit_last_week + self.education_last_week
    }

    pub fn reset_month(&mut self) {
        self.income_tax_month = 0;
        self.sales_tax_month = 0;
        self.dividend_tax_month = 0;
        self.benefits_month = 0;
        self.pensions_month = 0;
        self.child_benefit_month = 0;
        self.inheritance_tax_month = 0;
        self.education_month = 0;
        self.printed_month = 0;
        self.rent_month = 0;
        self.dividend_paid_month = 0;
        self.bond_interest_month = 0;
        self.bailouts_month = 0;
        self.issued_month = 0;
        self.issued_to_bank_month = 0;
        self.redeemed_month = 0;
    }

    /// Apply scheduled changes due at `month`. Returns descriptions of what changed.
    pub fn apply_schedule(&mut self, month: u32) -> Vec<String> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.schedule.len() {
            if self.schedule[i].0 <= month {
                let (_, k, v) = self.schedule.remove(i);
                match self.policy.set(&k, &v) {
                    Ok(()) => out.push(format!("{}={}", k, v)),
                    Err(e) => out.push(format!("ignored: {}", e)),
                }
            } else {
                i += 1;
            }
        }
        out
    }

    /// Gross price a consumer pays for one unit of good g at net price p.
    #[inline]
    pub fn gross_price(&self, g: usize, p: i64) -> (i64, i64) {
        let tax = (p as f64 * self.policy.sales_tax[g]).round() as i64;
        (p + tax, tax)
    }
}
