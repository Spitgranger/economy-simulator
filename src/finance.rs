//! Financial plumbing: dividends to shareholder homes, firm credit, the stock
//! market session, government bond financing and the bank's month.
//! Every function here is a set of ledger transfers plus, in the bank's case,
//! the only `mint`/`burn` calls outside world creation.

use crate::goods::{FOOD, SHELTER};
use crate::sim::{World, DAYS_PER_MONTH, WEEKS_PER_MONTH};

impl World {
    // ------------------------------------------------------------ dividends

    /// Pay `amount` from firm f to its shareholder homes pro rata, dividend tax withheld.
    pub(crate) fn distribute_dividend(&mut self, f: usize, amount: i64) {
        let total = self.firms.shares[f] as i64;
        if total == 0 {
            return;
        }
        let tax_rate = self.gov.policy.dividend_tax;
        let holders = std::mem::take(&mut self.firms.holders[f]);
        let mut paid = 0i64;
        for &(h, sh) in &holders {
            let part = amount * sh as i64 / total;
            if part <= 0 {
                continue;
            }
            let h = h as usize;
            let tax = (part as f64 * tax_rate).floor() as i64;
            self.ledger.transfer(self.firms.account[f], self.homes.account[h], part - tax, self.day);
            if tax > 0 {
                self.ledger.transfer(self.firms.account[f], self.gov.account, tax, self.day);
                self.gov.dividend_tax_month += tax;
                self.homes.tax_month[h] += tax;
            }
            self.homes.income_month[h] += part;
            self.homes.dividend_month[h] += part;
            paid += part;
        }
        self.firms.holders[f] = holders;
        self.month_dividends += paid;
        self.firms.dividend_ema[f] = 0.7 * self.firms.dividend_ema[f] + 0.3 * paid as f64;
    }

    // --------------------------------------------------------------- credit

    /// Monthly installment: interest to the bank, principal burned. False = cannot pay.
    pub(crate) fn service_loan(&mut self, f: usize) -> bool {
        let p = self.firms.loan_principal[f];
        if p == 0 {
            return true;
        }
        let months = self.firms.loan_months_left[f].max(1) as i64;
        let interest = (p as f64 * self.firms.loan_rate[f] / 12.0).round() as i64;
        let principal_due = if months == 1 { p } else { p / months };
        let cash = self.ledger.balance(self.firms.account[f]);
        if cash < interest + principal_due {
            return false;
        }
        if interest > 0 {
            self.ledger.transfer(self.firms.account[f], self.bank.account, interest, self.day);
            self.bank.interest_income_month += interest;
        }
        self.ledger.burn(self.firms.account[f], principal_due, self.day);
        self.bank.loans_total -= principal_due;
        self.firms.loan_principal[f] -= principal_due;
        self.firms.loan_months_left[f] -= 1;
        true
    }

    /// New money for firm f, limited by the bank's capacity and 3 months of the
    /// firm's revenue. Returns what was granted.
    pub(crate) fn borrow(&mut self, f: usize, amount: i64) -> i64 {
        let equity = self.ledger.balance(self.bank.account);
        let cap = self.bank.capacity(equity);
        let limit = 3 * self.firms.revenue_month[f] - self.firms.loan_principal[f];
        let grant = amount.min(cap).min(limit);
        if grant <= 0 {
            return 0;
        }
        self.ledger.mint(self.firms.account[f], grant);
        self.bank.loans_total += grant;
        self.bank.lent_month += grant;
        let p = self.firms.loan_principal[f];
        self.firms.loan_rate[f] = (p as f64 * self.firms.loan_rate[f] + grant as f64 * self.bank.loan_rate) / (p + grant) as f64;
        self.firms.loan_principal[f] = p + grant;
        self.firms.loan_months_left[f] = 12;
        grant
    }

    /// Default: the bank recovers what cash the firm has; the rest is a loss
    /// that burns the bank's own equity, shrinking its lending capacity.
    pub(crate) fn write_off(&mut self, f: usize) {
        let p = self.firms.loan_principal[f];
        if p == 0 {
            return;
        }
        let cash = self.ledger.balance(self.firms.account[f]);
        let recovered = cash.min(p);
        if recovered > 0 {
            self.ledger.burn(self.firms.account[f], recovered, self.day);
        }
        let loss = p - recovered;
        let equity = self.ledger.balance(self.bank.account);
        let absorbed = loss.min(equity);
        if absorbed > 0 {
            self.ledger.burn(self.bank.account, absorbed, self.day);
        }
        self.bank.loans_total -= p;
        self.bank.written_off_month += loss;
        self.bank.defaults_month += 1;
        self.firms.loan_principal[f] = 0;
        self.firms.loan_months_left[f] = 0;
        self.events.log(
            self.day,
            "default",
            &format!("firm{} defaults on {} cents: {} recovered, {} lost, bank equity now {}", f, p, recovered, loss, equity - absorbed),
        );
    }

    /// Residual cash of a closing firm to shareholder homes pro rata; shares vanish.
    pub(crate) fn clear_shareholders(&mut self, f: usize) {
        let cash = self.ledger.balance(self.firms.account[f]);
        let total = self.firms.shares[f] as i64;
        let holders = std::mem::take(&mut self.firms.holders[f]);
        for &(h, sh) in &holders {
            let part = if total > 0 { cash * sh as i64 / total } else { 0 };
            if part > 0 {
                self.ledger.transfer(self.firms.account[f], self.homes.account[h as usize], part, self.day);
            }
            self.homes.remove_shares(h as usize, f as u32, sh);
        }
        let rest = self.ledger.balance(self.firms.account[f]);
        if rest > 0 {
            let owner = self.firms.owner[f] as usize;
            let to = if owner < self.homes.n && self.homes.active[owner] { self.homes.account[owner] } else { self.gov.account };
            self.ledger.transfer(self.firms.account[f], to, rest, self.day);
        }
    }

    // --------------------------------------------------------- stock market

    /// Daily cost of a home's necessities at last prices (gross of tax).
    pub(crate) fn necessities_daily(&self, h: usize) -> f64 {
        self.homes.appetite_eff(h) * self.stats.last_prices[FOOD] + self.homes.shelter_eff(h) * self.stats.last_prices[SHELTER]
    }

    /// Move q shares of firm f from one home to another, both registers.
    pub(crate) fn transfer_shares(&mut self, f: usize, from: usize, to: usize, q: u32) {
        let removed = self.homes.remove_shares(from, f as u32, q);
        debug_assert_eq!(removed, q);
        let removed = self.firms.remove_holder(f, from as u32, q);
        debug_assert_eq!(removed, q);
        self.homes.add_shares(to, f as u32, q);
        self.firms.add_holder(f, to as u32, q);
    }

    /// Monthly: fundamental value per share, bond demand from the savings target,
    /// and portfolio marks. Trading itself happens weekly in `stock_session`.
    pub(crate) fn portfolio_month(&mut self) {
        let r_req = self.bank.policy_rate + 0.05;
        for &f in &self.firms.active_list {
            let f = f as usize;
            if self.firms.public[f] {
                continue;
            }
            let book = self.ledger.balance(self.firms.account[f]) + self.firms.inventory[f] * self.firms.price[f] - self.firms.loan_principal[f];
            let value = self.firms.dividend_ema[f] * 12.0 / r_req + book as f64;
            self.firms.fundamental[f] = (value / self.firms.shares[f].max(1) as f64).max(0.01);
        }
        let n = self.homes.n;
        self.bond_demand.clear();
        self.bond_demand.resize(n, 0);
        for h in 0..n {
            if !self.homes.has_adults(h) {
                continue;
            }
            let cash = self.ledger.balance(self.homes.account[h]);
            let stock_value = self.stock_value(h);
            let reserve = self.homes.patience[h] * DAYS_PER_MONTH as f64 * self.necessities_daily(h);
            let excess = cash + stock_value + self.homes.bonds[h] - reserve as i64;
            if (cash as f64) < 0.5 * reserve {
                self.bond_demand[h] = -self.homes.bonds[h];
            } else if excess > 0 {
                // higher rates make bonds more attractive
                let tilt = 1.0 + 4.0 * self.bank.policy_rate;
                let target_bond = ((self.homes.bond_share[h] * tilt).min(0.8) * excess as f64) as i64;
                self.bond_demand[h] = target_bond - self.homes.bonds[h];
            } else if self.homes.bonds[h] > 0 && (cash as f64) < reserve {
                self.bond_demand[h] = -self.homes.bonds[h].min(reserve as i64 - cash);
            }
        }
        self.mark_portfolios();
    }

    fn stock_value(&self, h: usize) -> i64 {
        self.homes.holdings[h].iter().map(|&(f, sh)| (sh as f64 * self.firms.share_price[f as usize]).round() as i64).sum()
    }

    fn mark_portfolios(&mut self) {
        let mut market_cap = 0i64;
        for &f in &self.firms.active_list {
            let f = f as usize;
            if !self.firms.public[f] {
                market_cap += (self.firms.shares[f] as f64 * self.firms.share_price[f]).round() as i64;
            }
        }
        self.market_cap = market_cap;
        for h in 0..self.homes.n {
            self.homes.assets_value[h] = self.homes.bonds[h] + self.stock_value(h);
        }
    }

    /// Weekly call auction in every firm's shares. Homes place bids and asks
    /// from three motives: rebalancing toward their equity target (hold band
    /// 85%-125%), value (price against fundamentals) and momentum (price
    /// against its slow average). Firms sell new shares when they need cash.
    /// Each firm clears at a price moved by the bid/ask ratio, capped at +-15%.
    pub(crate) fn stock_session(&mut self) {
        let n_homes = self.homes.n;
        let firms: Vec<u32> = self.firms.active_list.iter().copied().filter(|&f| !self.firms.public[f as usize]).collect();
        if firms.is_empty() {
            return;
        }
        for &f in &firms {
            let f = f as usize;
            self.firms.sell_book[f].clear();
            self.firms.bid_book[f].clear();
            if self.firms.issue_pending[f] > 0 {
                self.firms.sell_book[f].push((crate::firms::FIRM_SELLER, self.firms.issue_pending[f]));
            }
        }
        // signals per firm: value = ln(F/p), momentum = ln(p/ema)
        let mut sig_value = vec![0f64; self.firms.active.len()];
        let mut sig_mom = vec![0f64; self.firms.active.len()];
        for &f in &firms {
            let f = f as usize;
            let p = self.firms.share_price[f].max(0.01);
            sig_value[f] = (self.firms.fundamental[f].max(0.01) / p).ln().clamp(-1.0, 1.0);
            sig_mom[f] = (p / self.firms.price_ema[f].max(0.01)).ln().clamp(-1.0, 1.0);
        }

        // orders
        let mut scores: Vec<(f64, u32)> = Vec::with_capacity(firms.len());
        for h in 0..n_homes {
            if !self.homes.has_adults(h) {
                continue;
            }
            let cash = self.ledger.balance(self.homes.account[h]);
            let stock_value = self.stock_value(h);
            let reserve = self.homes.patience[h] * DAYS_PER_MONTH as f64 * self.necessities_daily(h);
            let wealth = (cash + stock_value + self.homes.bonds[h]) as f64 - reserve;
            let vw = self.homes.value_w[h];
            let mw = self.homes.momentum_w[h];
            let need_cash = (cash as f64) < 0.5 * reserve;
            let target = if wealth > 0.0 { self.homes.equity_share[h] * wealth } else { 0.0 };

            // asks: liquidity, rebalancing above the band, and negative signals
            let mut to_sell = if need_cash { stock_value } else if stock_value as f64 > 1.25 * target { stock_value - target as i64 } else { 0 };
            for k in 0..self.homes.holdings[h].len() {
                let (f, sh) = self.homes.holdings[h][k];
                let fu = f as usize;
                if !self.firms.active[fu] {
                    continue;
                }
                let price = self.firms.share_price[fu].max(0.01);
                let noise = (self.rng_finance.f64() - 0.5) * 0.1;
                let score = vw * sig_value[fu] + mw * sig_mom[fu] + noise;
                let mut q = 0u32;
                if to_sell > 0 {
                    q = ((to_sell as f64 / price).ceil() as u32).min(sh);
                    to_sell -= (q as f64 * price) as i64;
                }
                if score < -0.03 && q < sh {
                    q = q.max(((sh as f64 * (0.25 + (-score).min(0.5))) as u32).max(1)).min(sh);
                }
                if q > 0 {
                    self.firms.sell_book[fu].push((h as u32, q));
                }
            }

            // bids: rebalancing below the band plus an active budget on positive signals
            if need_cash || cash <= 0 {
                continue;
            }
            let rebalance = if (stock_value as f64) < 0.85 * target { target - stock_value as f64 } else { 0.0 };
            let active = 0.1 * (cash as f64 - reserve).max(0.0);
            if rebalance <= 0.0 && active <= 0.0 {
                continue;
            }
            scores.clear();
            for &f in &firms {
                let fu = f as usize;
                let noise = (self.rng_finance.f64() - 0.5) * 0.1;
                scores.push((vw * sig_value[fu] + mw * sig_mom[fu] + noise, f));
            }
            scores.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            let mut budget = rebalance;
            for &(score, _) in scores.iter().take(3) {
                if score > 0.02 {
                    budget += active * score.min(1.0) / 3.0;
                }
            }
            let budget = budget.min(cash as f64);
            if budget <= 0.0 {
                continue;
            }
            let picks: Vec<u32> = scores.iter().take(3).filter(|(sc, _)| *sc > -0.1 || rebalance > 0.0).map(|(_, f)| *f).collect();
            if picks.is_empty() {
                continue;
            }
            let per = budget / picks.len() as f64;
            for &f in &picks {
                let fu = f as usize;
                let q = (per / self.firms.share_price[fu].max(0.01)).floor().min(u32::MAX as f64 / 4.0) as u32;
                if q > 0 {
                    self.firms.bid_book[fu].push((h as u32, q));
                }
            }
        }

        // clearing, firm by firm
        let mut volume = 0u32;
        let mut value_traded = 0i64;
        for &f in &firms {
            let f = f as usize;
            let bids: u64 = self.firms.bid_book[f].iter().map(|&(_, q)| q as u64).sum();
            let asks: u64 = self.firms.sell_book[f].iter().map(|&(_, q)| q as u64).sum();
            let p_old = self.firms.share_price[f].max(0.01);
            let p_new = if bids > 0 && asks > 0 {
                let ratio = (bids as f64 / asks as f64).powf(0.25).clamp(0.92, 1.08);
                (p_old * ratio).max(0.01)
            } else if bids > 0 {
                (p_old * 1.04).max(0.01)
            } else if asks > 0 {
                (p_old * 0.96).max(0.01)
            } else {
                // no interest either way: drift toward fundamentals
                (0.9 * p_old + 0.1 * self.firms.fundamental[f]).max(0.01)
            };
            self.firms.share_price[f] = p_new;
            if bids > 0 && asks > 0 {
                let mut bid_book = std::mem::take(&mut self.firms.bid_book[f]);
                let mut ask_book = std::mem::take(&mut self.firms.sell_book[f]);
                self.rng_finance.shuffle(&mut bid_book);
                let (mut bi, mut ai) = (0usize, 0usize);
                while bi < bid_book.len() && ai < ask_book.len() {
                    let (buyer, bq) = bid_book[bi];
                    let (seller, aq) = ask_book[ai];
                    let buyer = buyer as usize;
                    let affordable = (self.ledger.balance(self.homes.account[buyer]) as f64 / p_new).floor().min(u32::MAX as f64 / 4.0) as u32;
                    let q = bq.min(aq).min(affordable);
                    if q == 0 {
                        bi += 1; // buyer cannot pay at the clearing price
                        continue;
                    }
                    let amt = (q as f64 * p_new).round() as i64;
                    if seller == crate::firms::FIRM_SELLER {
                        // new shares: proceeds to the firm, register grows
                        self.ledger.transfer(self.homes.account[buyer], self.firms.account[f], amt, self.day);
                        self.firms.shares[f] += q;
                        self.firms.add_holder(f, buyer as u32, q);
                        self.homes.add_shares(buyer, f as u32, q);
                        self.firms.issue_pending[f] -= q;
                        self.month_equity_issued += amt;
                    } else if seller as usize != buyer {
                        self.ledger.transfer(self.homes.account[buyer], self.homes.account[seller as usize], amt, self.day);
                        self.homes.income_month[seller as usize] += amt;
                        self.transfer_shares(f, seller as usize, buyer, q);
                    }
                    volume += q;
                    value_traded += amt;
                    bid_book[bi].1 -= q;
                    ask_book[ai].1 -= q;
                    if bid_book[bi].1 == 0 {
                        bi += 1;
                    }
                    if ask_book[ai].1 == 0 {
                        ai += 1;
                    }
                }
                self.firms.bid_book[f] = bid_book;
                self.firms.sell_book[f] = ask_book;
            }
            self.firms.price_ema[f] = 0.9 * self.firms.price_ema[f] + 0.1 * p_new;
            if p_new > 500.0 {
                self.split_shares(f, 10);
            } else if p_new < 1.0 && self.firms.shares[f] >= 1000 {
                self.reverse_split(f, 10);
            }
        }
        self.month_stock_volume += volume;
        self.month_value_traded += value_traded;
        self.mark_portfolios();
    }

    /// Stock split: more shares, same wealth, affordable lots.
    fn split_shares(&mut self, f: usize, factor: u32) {
        self.firms.shares[f] *= factor;
        self.firms.share_price[f] /= factor as f64;
        self.firms.price_ema[f] /= factor as f64;
        self.firms.fundamental[f] /= factor as f64;
        self.firms.issue_pending[f] *= factor;
        for k in 0..self.firms.holders[f].len() {
            let (h, q) = self.firms.holders[f][k];
            self.firms.holders[f][k].1 = q * factor;
            if let Some(e) = self.homes.holdings[h as usize].iter_mut().find(|e| e.0 == f as u32) {
                e.1 *= factor;
            }
        }
    }

    /// Reverse split: fewer shares, ten times the price; odd lots below one new
    /// share are cancelled (worth less than a cent each).
    fn reverse_split(&mut self, f: usize, factor: u32) {
        let mut total = 0u32;
        let holders = std::mem::take(&mut self.firms.holders[f]);
        let mut kept = Vec::with_capacity(holders.len());
        for &(h, q) in &holders {
            let nq = q / factor;
            if let Some(pos) = self.homes.holdings[h as usize].iter().position(|e| e.0 == f as u32) {
                if nq == 0 {
                    self.homes.holdings[h as usize].swap_remove(pos);
                } else {
                    self.homes.holdings[h as usize][pos].1 = nq;
                }
            }
            if nq > 0 {
                kept.push((h, nq));
                total += nq;
            }
        }
        self.firms.holders[f] = kept;
        self.firms.shares[f] = total.max(1);
        self.firms.share_price[f] *= factor as f64;
        self.firms.price_ema[f] *= factor as f64;
        self.firms.fundamental[f] *= factor as f64;
        self.firms.issue_pending[f] /= factor;
    }

    /// A cash-squeezed firm offers new shares worth `need` at the current price,
    /// at most a fifth of its shares a month.
    pub(crate) fn request_equity(&mut self, f: usize, need: i64) {
        let price = self.firms.share_price[f].max(0.01);
        let q = ((need as f64 / price) as u32).min(self.firms.shares[f] / 5);
        if q > 0 {
            self.firms.issue_pending[f] = self.firms.issue_pending[f].max(q);
        }
    }

    // ------------------------------------------------------ government debt

    fn annual_tax(&self) -> i64 {
        self.stats.months.iter().rev().take(11).map(|m| m.taxes).sum::<i64>() + self.gov.tax_month()
    }

    /// Money creation for the treasury: the standing print rate plus any one-off
    /// print or burn the player asked for. The bank mints; the invariant holds.
    fn print_money(&mut self) {
        let steady = (self.gov.policy.print_rate.max(0.0) * self.ledger.money_supply() as f64).round() as i64;
        let mut amount = steady + self.gov.pending_print;
        self.gov.pending_print = 0;
        if amount > 0 {
            self.ledger.mint(self.gov.account, amount);
        } else if amount < 0 {
            let burn = (-amount).min(self.ledger.balance(self.gov.account));
            self.ledger.burn(self.gov.account, burn, self.day);
            amount = -burn;
        }
        if amount != 0 {
            self.gov.printed_month += amount;
            self.bank.printed_total += amount;
            if steady != amount || amount.abs() > self.ledger.money_supply() / 50 {
                self.events.log(self.day, "money", &format!("treasury {} {} cents; money supply now {}", if amount > 0 { "printed" } else { "burned" }, amount.abs(), self.ledger.money_supply()));
            }
        }
    }

    /// Bond auction, redemptions, coupon payments, debt brake or universal dividend.
    pub(crate) fn government_finance(&mut self, month: u32) {
        self.print_money();
        let n = self.homes.n;
        let annual_tax = self.annual_tax();
        let rate_m = self.gov.bond_rate / 12.0;
        let interest_due = (self.gov.debt as f64 * rate_m).round() as i64;
        let maturing: i64 = self.gov.issues.iter().filter(|(m, _)| *m <= month).map(|(_, f)| *f).sum();
        let benefits_next = WEEKS_PER_MONTH as i64 * self.gov.transfers_last_week();
        let bank_equity = self.ledger.balance(self.bank.account);
        let recap = if self.bank.credit_enabled && bank_equity < self.bank.target_equity / 10 {
            self.bank.target_equity / 2 - bank_equity
        } else {
            0
        };
        let treasury = self.ledger.balance(self.gov.account);
        let need = interest_due + maturing + benefits_next + recap - treasury;

        if need > 0 {
            let mut remaining = need;
            self.job_seekers.clear();
            self.job_seekers.extend((0..n as u32).filter(|&h| self.bond_demand[h as usize] > 0));
            self.rng_finance.shuffle(&mut self.job_seekers);
            for k in 0..self.job_seekers.len() {
                if remaining <= 0 {
                    break;
                }
                let h = self.job_seekers[k] as usize;
                let buy = self.bond_demand[h].min(remaining).min(self.ledger.balance(self.homes.account[h]));
                if buy <= 0 {
                    continue;
                }
                self.ledger.transfer(self.homes.account[h], self.gov.account, buy, self.day);
                self.homes.bonds[h] += buy;
                self.bond_demand[h] -= buy;
                remaining -= buy;
            }
            if remaining > 0 {
                let cap = self.bank.capacity(self.ledger.balance(self.bank.account));
                let buy = remaining.min(cap);
                if buy > 0 {
                    self.ledger.mint(self.gov.account, buy);
                    self.bank.bonds += buy;
                    self.gov.issued_to_bank_month += buy;
                    remaining -= buy;
                }
            }
            let issued = need - remaining;
            if issued > 0 {
                self.gov.issues.push((month + 12, issued));
                self.gov.debt += issued;
                self.gov.issued_month += issued;
            }
            if remaining > 0 {
                self.events.log(self.day, "treasury", &format!("could not place {} cents of bonds; transfers will be cut pro rata", remaining));
            }
        }

        if recap > 0 {
            let amt = recap.min(self.ledger.balance(self.gov.account));
            if amt > 0 {
                self.ledger.transfer(self.gov.account, self.bank.account, amt, self.day);
                self.gov.bailouts_month += amt;
                self.events.log(self.day, "bailout", &format!("treasury recapitalizes the bank with {} cents (equity was {})", amt, bank_equity));
            }
        }

        if maturing > 0 && self.gov.debt > 0 {
            let avail = self.ledger.balance(self.gov.account).min(maturing);
            let debt = self.gov.debt;
            let mut paid = 0i64;
            for h in 0..n {
                let b = self.homes.bonds[h];
                if b == 0 {
                    continue;
                }
                let part = ((avail as i128 * b as i128) / debt as i128) as i64;
                if part > 0 {
                    self.ledger.transfer(self.gov.account, self.homes.account[h], part, self.day);
                    self.homes.bonds[h] -= part;
                    paid += part;
                }
            }
            let part = ((avail as i128 * self.bank.bonds as i128) / debt as i128) as i64;
            if part > 0 {
                self.ledger.burn(self.gov.account, part, self.day);
                self.bank.bonds -= part;
                paid += part;
            }
            self.gov.debt -= paid;
            self.gov.redeemed_month += paid;
            self.gov.issues.retain(|(m, _)| *m > month);
            let rolled = maturing - paid;
            if rolled > 0 {
                self.gov.issues.push((month + 12, rolled));
            }
        }

        if self.gov.debt > 0 && rate_m > 0.0 {
            let mut total = 0i64;
            for h in 0..n {
                let c = (self.homes.bonds[h] as f64 * rate_m).floor() as i64;
                if c > 0 && self.ledger.balance(self.gov.account) >= c {
                    self.ledger.transfer(self.gov.account, self.homes.account[h], c, self.day);
                    self.homes.interest_month[h] += c;
                    self.homes.income_month[h] += c;
                    total += c;
                }
            }
            let c = (self.bank.bonds as f64 * rate_m).floor() as i64;
            if c > 0 && self.ledger.balance(self.gov.account) >= c {
                self.ledger.transfer(self.gov.account, self.bank.account, c, self.day);
                self.bank.interest_income_month += c;
                total += c;
            }
            self.gov.bond_interest_month = total;
        }

        for h in 0..n {
            let d = self.bond_demand[h];
            if d >= 0 || self.homes.bonds[h] == 0 {
                continue;
            }
            let cap = self.bank.capacity(self.ledger.balance(self.bank.account));
            let sell = (-d).min(self.homes.bonds[h]).min(cap);
            if sell > 0 {
                self.ledger.mint(self.homes.account[h], sell);
                self.homes.bonds[h] -= sell;
                self.bank.bonds += sell;
            }
        }

        let brake = self.gov.debt > 0 && self.gov.debt > 2 * annual_tax.max(1);
        if brake != self.gov.debt_brake {
            self.events.log(self.day, "treasury", &format!("debt brake {} (debt {} vs annual tax {})", if brake { "on" } else { "off" }, self.gov.debt, annual_tax));
        }
        self.gov.debt_brake = brake;
        let reserve = WEEKS_PER_MONTH as i64 * self.gov.transfers_last_week() + interest_due;
        if brake {
            let surplus = self.ledger.balance(self.gov.account) - reserve;
            if surplus > 0 {
                self.buy_back_bonds(surplus.min(self.gov.debt));
            }
        } else {
            self.universal_dividend(reserve);
        }

        self.gov.bond_rate = self.bank.policy_rate + 0.01 + 0.02 * (self.gov.debt as f64 / annual_tax.max(1) as f64).min(3.0);
        self.check_debt_books();
    }

    fn buy_back_bonds(&mut self, amount: i64) {
        let debt = self.gov.debt;
        if debt == 0 || amount <= 0 {
            return;
        }
        let mut paid = 0i64;
        for h in 0..self.homes.n {
            let b = self.homes.bonds[h];
            if b == 0 {
                continue;
            }
            let part = ((amount as i128 * b as i128) / debt as i128) as i64;
            if part > 0 {
                self.ledger.transfer(self.gov.account, self.homes.account[h], part, self.day);
                self.homes.bonds[h] -= part;
                paid += part;
            }
        }
        let part = ((amount as i128 * self.bank.bonds as i128) / debt as i128) as i64;
        if part > 0 {
            self.ledger.burn(self.gov.account, part, self.day);
            self.bank.bonds -= part;
            paid += part;
        }
        if paid == 0 {
            return;
        }
        self.gov.debt -= paid;
        self.gov.redeemed_month += paid;
        let new_debt = self.gov.debt;
        let mut acc = 0i64;
        for issue in self.gov.issues.iter_mut() {
            issue.1 = ((issue.1 as i128 * new_debt as i128) / debt as i128) as i64;
            acc += issue.1;
        }
        if let Some(first) = self.gov.issues.first_mut() {
            first.1 += new_debt - acc;
        }
        self.gov.issues.retain(|(_, f)| *f > 0);
    }

    fn check_debt_books(&self) {
        let held: i64 = self.homes.bonds.iter().sum::<i64>() + self.bank.bonds;
        let faces: i64 = self.gov.issues.iter().map(|(_, f)| *f).sum();
        assert_eq!(held, self.gov.debt, "day {}: bonds held {} != debt {}", self.day, held, self.gov.debt);
        assert_eq!(faces, self.gov.debt, "day {}: issue faces {} != debt {}", self.day, faces, self.gov.debt);
    }

    // ----------------------------------------------------------------- bank

    /// Rates from inflation; profit above the equity target paid to homes
    /// pro rata to their cash, which is the deposit rate this economy has.
    pub(crate) fn bank_month(&mut self) {
        let manual = if self.gov.policy.policy_rate >= 0.0 { Some(self.gov.policy.policy_rate) } else { None };
        self.bank.set_rates(self.stats.last_cpi, manual);
        let target = self.bank.target_equity.max(self.bank.loans_total / 5);
        let excess = self.ledger.balance(self.bank.account) - target;
        if excess <= 0 {
            return;
        }
        let total: i64 = (0..self.homes.n).filter(|&h| self.homes.active[h]).map(|h| self.ledger.balance(self.homes.account[h])).sum();
        if total <= 0 {
            return;
        }
        let mut paid = 0i64;
        for h in 0..self.homes.n {
            if !self.homes.active[h] {
                continue;
            }
            let c = ((excess as i128 * self.ledger.balance(self.homes.account[h]) as i128) / total as i128) as i64;
            if c > 0 {
                self.ledger.transfer(self.bank.account, self.homes.account[h], c, self.day);
                self.homes.interest_month[h] += c;
                self.homes.income_month[h] += c;
                paid += c;
            }
        }
        self.bank.deposit_interest_month = paid;
    }
}
