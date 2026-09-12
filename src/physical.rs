//! Daily reconciliation of the three goods stored by producers.
//!
//! Construction delivery is an irreversible allocation (a sink here), not a
//! reusable project inventory. Completed buildings are outside this unit ledger.
//! School seats are a monthly capacity measure and are deliberately excluded.
use crate::{
    firms::Firms,
    goods::{EDUCATION, GOODS},
};

/// Unit flows for the most recently completed tick, indexed by goods constants.
/// This report is serialized with the world and uses integer totals wide enough
/// to sum every firm's i64 inventory without overflowing an i64 aggregate.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PhysicalGoodsBalance {
    pub day: u32,
    pub opening: [i128; EDUCATION],
    pub produced: [i128; EDUCATION],
    pub household_consumption: [i128; EDUCATION],
    pub construction_allocated: [i128; EDUCATION],
    pub closure_losses: [i128; EDUCATION],
    pub closing: [i128; EDUCATION],
}

impl PhysicalGoodsBalance {
    pub(crate) fn begin(firms: &Firms, day: u32) -> Self {
        Self {
            day,
            opening: stocks(firms),
            ..Self::default()
        }
    }

    pub(crate) fn finish(&mut self, firms: &Firms) {
        self.closing = stocks(firms);
        self.assert_conserved();
    }

    /// Reconciliation runs in release builds as well as debug builds.
    pub fn assert_conserved(&self) {
        for g in 0..EDUCATION {
            assert!(
                self.produced[g] >= 0
                    && self.household_consumption[g] >= 0
                    && self.construction_allocated[g] >= 0
                    && self.closure_losses[g] >= 0,
                "negative physical goods flow on day {} for {}",
                self.day,
                GOODS[g].name
            );
            let expected = self.opening[g] + self.produced[g]
                - self.household_consumption[g]
                - self.construction_allocated[g]
                - self.closure_losses[g];
            assert_eq!(self.closing[g], expected,
                "physical goods not conserved on day {} for {}: opening {}, production {}, household consumption {}, construction allocation {}, closure losses {}",
                self.day, GOODS[g].name, self.opening[g], self.produced[g],
                self.household_consumption[g], self.construction_allocated[g], self.closure_losses[g]);
        }
    }
}

fn stocks(firms: &Firms) -> [i128; EDUCATION] {
    let mut totals = [0; EDUCATION];
    // Check all slots so forgotten inventory on a deactivated firm cannot hide
    // outside active_list, and negative stock cannot cancel a positive stock.
    for f in 0..firms.inventory.len() {
        let g = firms.good[f] as usize;
        if g == EDUCATION {
            continue;
        }
        assert!(g < EDUCATION, "invalid physical good at firm {f}");
        assert!(
            firms.inventory[f] >= 0,
            "negative physical inventory at firm {f}"
        );
        assert!(
            firms.active[f] || firms.inventory[f] == 0,
            "inactive firm {f} retains physical inventory"
        );
        totals[g] += firms.inventory[f] as i128;
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goods::{FOOD, SHELTER};

    fn firms() -> Firms {
        let mut firms = Firms::new(3, &[0, 1, 2]);
        firms.activate(0, FOOD, 1.0, 1, 1, 1, 0.0, 1).unwrap();
        firms.activate(0, SHELTER, 1.0, 1, 1, 1, 0.0, 1).unwrap();
        firms.activate(0, EDUCATION, 1.0, 1, 1, 1, 0.0, 1).unwrap();
        firms
    }

    #[test]
    fn unexplained_creation_and_destruction_fail_reconciliation() {
        for delta in [-1, 1] {
            let mut firms = firms();
            let f = firms.active_list[0] as usize;
            firms.inventory[f] = 10;
            let mut balance = PhysicalGoodsBalance::begin(&firms, 1);
            firms.inventory[f] += delta;
            assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || balance.finish(&firms)
            ))
            .is_err());
        }
    }

    #[test]
    fn education_seats_are_not_physical_stock() {
        let mut firms = firms();
        let f = firms.active_list[2] as usize;
        firms.inventory[f] = 100;
        let mut balance = PhysicalGoodsBalance::begin(&firms, 1);
        firms.inventory[f] = 2;
        balance.finish(&firms);
        assert_eq!(balance.closing, [0; EDUCATION]);
    }

    #[test]
    fn negative_and_orphaned_stock_are_rejected() {
        for (active, inventory) in [(true, -1), (false, 1)] {
            let mut firms = firms();
            let f = firms.active_list[0] as usize;
            firms.active[f] = active;
            firms.inventory[f] = inventory;
            assert!(std::panic::catch_unwind(|| PhysicalGoodsBalance::begin(&firms, 1)).is_err());
        }
    }
}
