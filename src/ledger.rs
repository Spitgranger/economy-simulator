//! Double-entry money ledger. All money in the world lives here as integer
//! cents. Money enters only through `mint` (the initial endowment and the
//! bank's lending) and leaves only through `burn` (loan repayment and bank
//! losses). Everything else is a transfer, and the sum of all balances must
//! always equal minted minus burned.

pub type Account = u32;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Ledger {
    balances: Vec<i64>,
    minted: i64,
    burned: i64,
    pub transfers: u64,
}

impl Ledger {
    pub fn new() -> Ledger {
        Ledger { balances: Vec::new(), minted: 0, burned: 0, transfers: 0 }
    }

    pub fn open(&mut self) -> Account {
        self.balances.push(0);
        (self.balances.len() - 1) as Account
    }

    /// Money creation. Only the initial endowment and the bank may call this.
    pub fn mint(&mut self, to: Account, amount: i64) {
        assert!(amount >= 0, "mint of negative amount {}", amount);
        self.balances[to as usize] += amount;
        self.minted += amount;
    }

    /// Money destruction: loan principal repaid, or bank equity absorbing a loss.
    pub fn burn(&mut self, from: Account, amount: i64, day: u32) {
        assert!(amount >= 0, "day {}: burn of negative amount {}", day, amount);
        let fb = self.balances[from as usize];
        assert!(fb >= amount, "day {}: account {} cannot burn {} (has {})", day, from, amount, fb);
        self.balances[from as usize] = fb - amount;
        self.burned += amount;
    }

    /// Money in circulation: everything minted that has not been burned.
    pub fn money_supply(&self) -> i64 {
        self.minted - self.burned
    }

    #[inline]
    pub fn balance(&self, a: Account) -> i64 {
        self.balances[a as usize]
    }

    #[inline]
    pub fn transfer(&mut self, from: Account, to: Account, amount: i64, day: u32) {
        assert!(amount >= 0, "day {}: negative transfer {} from {} to {}", day, amount, from, to);
        assert!(from != to, "day {}: self-transfer on account {}", day, from);
        let fb = self.balances[from as usize];
        assert!(
            fb >= amount,
            "day {}: account {} overdrawn: has {} cents, tried to pay {} to {}",
            day, from, fb, amount, to
        );
        self.balances[from as usize] = fb - amount;
        self.balances[to as usize] += amount;
        self.transfers += 1;
    }

    pub fn total(&self) -> i64 {
        self.balances.iter().sum()
    }

    pub fn minted(&self) -> i64 {
        self.minted
    }

    /// The invariant that catches most bugs: money is neither created nor
    /// destroyed anywhere except `mint` and `burn`.
    pub fn assert_conserved(&self, day: u32) {
        let total = self.total();
        assert_eq!(
            total, self.minted - self.burned,
            "day {}: money not conserved: balances sum to {} but minted {} - burned {} = {}",
            day, total, self.minted, self.burned, self.minted - self.burned
        );
    }
}
