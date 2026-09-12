//! The banking system, consolidated into one bank. It is the only module that
//! creates money (lending to firms, buying government bonds) or destroys it
//! (repayment; its own equity absorbing a loss). Its cash account is its
//! equity; lending capacity is a multiple of it, so losses tighten credit.

use crate::ledger::Account;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Bank {
    pub account: Account,
    pub target_equity: i64,
    pub leverage: f64,
    pub credit_enabled: bool,

    pub loans_total: i64,
    pub bonds: i64, // government bonds held, face value

    pub policy_rate: f64, // annual
    pub loan_rate: f64,
    pub loss_rate_ema: f64,
    pub cpi_history: Vec<f64>,
    pub inflation: f64,

    // monthly counters
    pub lent_month: i64,
    pub interest_income_month: i64,
    pub written_off_month: i64,
    pub defaults_month: u32,
    pub deposit_interest_month: i64,
    pub printed_total: i64, // net money created for the treasury outside lending
}

impl Bank {
    pub fn new(account: Account, target_equity: i64, credit_enabled: bool) -> Bank {
        Bank {
            account,
            target_equity,
            leverage: 10.0,
            credit_enabled,
            loans_total: 0,
            bonds: 0,
            policy_rate: 0.02,
            loan_rate: 0.05,
            loss_rate_ema: 0.0,
            cpi_history: Vec::new(),
            inflation: 0.0,
            lent_month: 0,
            interest_income_month: 0,
            written_off_month: 0,
            defaults_month: 0,
            deposit_interest_month: 0,
            printed_total: 0,
        }
    }

    /// How much more the bank may lend given its equity (cash) and claims.
    pub fn capacity(&self, equity: i64) -> i64 {
        if !self.credit_enabled {
            return 0;
        }
        ((self.leverage * equity as f64) as i64 - self.loans_total - self.bonds).max(0)
    }

    /// Policy rate from a Taylor-style rule on 12-month CPI inflation, unless
    /// the government sets it by hand; the loan rate adds a spread and a premium
    /// for recent losses.
    pub fn set_rates(&mut self, cpi: f64, manual: Option<f64>) {
        self.cpi_history.push(cpi);
        let n = self.cpi_history.len();
        self.inflation = if n > 12 { cpi / self.cpi_history[n - 13] - 1.0 } else { 0.0 };
        self.policy_rate = match manual {
            Some(r) => r.clamp(0.0, 0.5),
            None => (0.02 + 1.5 * (self.inflation - 0.02)).clamp(0.0, 0.25),
        };
        let exposure = (self.loans_total + self.written_off_month).max(1) as f64;
        self.loss_rate_ema = 0.9 * self.loss_rate_ema + 0.1 * (self.written_off_month as f64 / exposure);
        self.loan_rate = self.policy_rate + 0.03 + 2.0 * self.loss_rate_ema;
    }

    pub fn reset_month(&mut self) {
        self.lent_month = 0;
        self.interest_income_month = 0;
        self.written_off_month = 0;
        self.defaults_month = 0;
        self.deposit_interest_month = 0;
    }
}
