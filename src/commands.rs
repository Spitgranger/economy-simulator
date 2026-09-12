//! Frontend-independent player decisions and real-time simulation pacing.
use crate::{city::Zone, sim::World};
use std::{fmt, time::Duration};

#[derive(Clone, Debug)]
pub enum Command {
    Build {
        x: usize,
        y: usize,
        zone: Zone,
    },
    /// Cancel a queued order or refund unspent escrow from an active project.
    CancelBuild {
        x: usize,
        y: usize,
    },
    CancelAllQueuedBuilds,
    SetPolicy {
        key: String,
        value: String,
    },
    PrintMoney {
        amount: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutcome {
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandError(pub String);
impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CommandError {}
fn invalid(message: &str) -> CommandError {
    CommandError(message.into())
}

impl World {
    pub fn start_player_session(&mut self) {
        if self.gov.player_policy.is_some() {
            return;
        }
        self.gov.player_policy = Some(self.gov.policy.clone());
        self.gov.player_in_power = true;
        self.events.log(
            self.day,
            "player",
            "interactive mode: govern the region; elections judge your platform",
        );
    }

    /// Read-only construction preview; uses the same checks as command submission.
    pub fn preview_build(&self, x: usize, y: usize, zone: Zone) -> Result<(), CommandError> {
        if x >= self.city.w || y >= self.city.h {
            return Err(invalid("site is outside the city"));
        }
        let tile = self.city.idx(x, y);
        let current = &self.city.tiles[tile as usize];
        if current.zone == zone {
            return Err(invalid("site already has this zone"));
        }
        if current.occupants > 0 {
            return Err(invalid("occupied sites cannot be replaced"));
        }
        if self
            .gov
            .pending_builds
            .iter()
            .any(|&(px, py, _)| (px, py) == (x, y))
        {
            return Err(invalid("site already has a queued order"));
        }
        if self.construction.iter().any(|project| project.tile == tile) {
            return Err(invalid("site has active construction"));
        }
        let reserved = self
            .gov
            .pending_builds
            .iter()
            .try_fold(0i64, |sum, &(_, _, z)| {
                sum.checked_add(Zone::from_u8(z).cost())
            })
            .ok_or_else(|| invalid("queued construction budget overflow"))?;
        let total = reserved
            .checked_add(zone.cost())
            .ok_or_else(|| invalid("construction budget overflow"))?;
        if zone.cost() > 0 && total > self.ledger.balance(self.gov.account) {
            return Err(invalid(
                "treasury cannot cover this order and existing queued orders",
            ));
        }
        Ok(())
    }

    /// Validate entirely before mutating. Fiscal/build authority retains the
    /// existing sandbox behavior even in opposition; policies become a platform.
    pub fn apply_command(&mut self, command: Command) -> Result<CommandOutcome, CommandError> {
        let message = match command {
            Command::Build { x, y, zone } => {
                self.preview_build(x, y, zone)?;
                self.gov.pending_builds.push((x, y, zone as u8));
                format!(
                    "queued {} at ({x},{y}) for month-end authorization",
                    zone.name()
                )
            }
            Command::CancelBuild { x, y } => {
                if let Some(index) = self
                    .gov
                    .pending_builds
                    .iter()
                    .position(|&(px, py, _)| (px, py) == (x, y))
                {
                    self.gov.pending_builds.remove(index);
                    format!("cancelled queued order at ({x},{y})")
                } else {
                    let refund = self.cancel_construction(x, y).map_err(CommandError)?;
                    format!("cancelled active project at ({x},{y}); refunded {refund} cents; delivered materials and completed work cannot be recovered")
                }
            }
            Command::CancelAllQueuedBuilds => {
                let count = self.gov.pending_builds.len();
                if count == 0 {
                    return Err(invalid("there are no queued orders"));
                }
                self.gov.pending_builds.clear();
                format!("cancelled {count} queued orders")
            }
            Command::PrintMoney { amount } => {
                if amount == 0 {
                    return Err(invalid("money order must be nonzero"));
                }
                let pending = self
                    .gov
                    .pending_print
                    .checked_add(amount)
                    .filter(|&v| v != i64::MIN)
                    .ok_or_else(|| invalid("money order overflow"))?;
                // The ledger uses signed cents; reject an order that cannot be
                // represented against the current treasury and total supply.
                if pending > 0 {
                    self.ledger
                        .balance(self.gov.account)
                        .checked_add(pending)
                        .ok_or_else(|| invalid("treasury overflow"))?;
                    self.ledger
                        .minted()
                        .checked_add(pending)
                        .ok_or_else(|| invalid("money supply overflow"))?;
                }
                self.gov.pending_print = pending;
                format!("queued money change of {amount} cents at month end")
            }
            Command::SetPolicy { key, value } => {
                validate_policy(&key, &value)?;
                let mut policy = self
                    .gov
                    .player_policy
                    .clone()
                    .unwrap_or_else(|| self.gov.policy.clone());
                policy.set(&key, &value).map_err(CommandError)?;
                if self.gov.player_in_power {
                    self.gov.policy = policy.clone();
                }
                self.gov.player_policy = Some(policy);
                if self.gov.player_in_power {
                    format!("government sets {key}={value}")
                } else {
                    format!("opposition platform sets {key}={value}; takes effect after an election win")
                }
            }
        };
        self.events.log(self.day, "player", &message);
        Ok(CommandOutcome { message })
    }
}

fn validate_policy(key: &str, value: &str) -> Result<(), CommandError> {
    if key == "policy-rate" && value == "auto" {
        return Ok(());
    }
    let number = value
        .parse::<f64>()
        .map_err(|_| invalid("policy requires a number"))?;
    if !number.is_finite() {
        return Err(invalid("policy value must be finite"));
    }
    let valid = match key {
        "income-tax" | "sales-tax" | "luxury-tax" | "dividend-tax" | "inheritance-tax"
        | "surplus-dividend" | "ground-rent" => (0.0..=1.0).contains(&number),
        "benefit" | "basic-income" | "pension" | "child-benefit" | "teacher-pay" => {
            (0.0..=100.0).contains(&number)
        }
        "class-size" => (0.0..=10000.0).contains(&number),
        "min-wage" => number >= 0.0 && number < i64::MAX as f64 && number.fract() == 0.0,
        "policy-rate" => (-1.0..=1.0).contains(&number),
        "print-rate" => (-1.0..=1.0).contains(&number),
        _ => return Err(invalid("unknown policy")),
    };
    if valid {
        Ok(())
    } else {
        Err(invalid("policy value is outside its supported range"))
    }
}

/// Fixed-day stepping with bounded work per render/update. Excess elapsed time
/// remains as debt, so slow frames do not silently change simulated speed.
#[derive(Clone, Debug)]
pub struct SimulationClock {
    carry_days: f64,
    steps: u64,
    max_days_per_update: u32,
}
impl SimulationClock {
    pub fn new(max_days_per_update: u32) -> Self {
        Self {
            carry_days: 0.0,
            steps: 0,
            max_days_per_update: max_days_per_update.max(1),
        }
    }
    pub fn advance(&mut self, elapsed: Duration, speed: f64, paused: bool, step_days: u32) -> u32 {
        self.steps = self.steps.saturating_add(step_days as u64);
        let manual = self.steps.min(self.max_days_per_update as u64) as u32;
        self.steps -= manual as u64;
        let capacity = self.max_days_per_update - manual;
        if paused {
            return manual;
        }
        if speed == f64::INFINITY {
            return manual + capacity;
        }
        if speed.is_finite() && speed > 0.0 {
            self.carry_days =
                (self.carry_days + elapsed.as_secs_f64() * speed).min(u32::MAX as f64);
        }
        let elapsed_days = ((self.carry_days + 1e-9).floor() as u64).min(capacity as u64) as u32;
        self.carry_days = (self.carry_days - elapsed_days as f64).max(0.0);
        manual + elapsed_days
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Config;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn world() -> World {
        World::new(Config {
            n_hh: 20,
            n_firms: 4,
            firm_cap: 20,
            quiet: true,
            out_dir: std::env::temp_dir()
                .join(format!(
                    "econsim-command-tests-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ))
                .to_string_lossy()
                .into_owned(),
            ..Config::default()
        })
        .unwrap()
    }
    #[test]
    fn invalid_commands_leave_state_unchanged() {
        let mut w = world();
        w.start_player_session();
        let policy = w.gov.policy.clone();
        let cash = w.ledger.balance(w.gov.account);
        for cmd in [
            Command::Build {
                x: usize::MAX,
                y: 0,
                zone: Zone::Park,
            },
            Command::SetPolicy {
                key: "income-tax".into(),
                value: "NaN".into(),
            },
            Command::SetPolicy {
                key: "income-tax".into(),
                value: "1.1".into(),
            },
            Command::SetPolicy {
                key: "min-wage".into(),
                value: "1e40".into(),
            },
            Command::SetPolicy {
                key: "unknown".into(),
                value: "1".into(),
            },
            Command::PrintMoney { amount: i64::MIN },
        ] {
            assert!(w.apply_command(cmd).is_err());
        }
        assert_eq!(policy, w.gov.policy);
        assert_eq!(Some(policy), w.gov.player_policy);
        assert_eq!(cash, w.ledger.balance(w.gov.account));
        assert!(w.gov.pending_builds.is_empty());
        assert_eq!(0, w.gov.pending_print);
    }
    #[test]
    fn policy_obeys_elected_authority_and_resume_preserves_opposition() {
        let mut w = world();
        w.start_player_session();
        w.gov.player_in_power = false;
        let incumbent = w.gov.policy.clone();
        w.apply_command(Command::SetPolicy {
            key: "income-tax".into(),
            value: "0.3".into(),
        })
        .unwrap();
        w.start_player_session();
        assert!(!w.gov.player_in_power);
        assert_eq!(incumbent, w.gov.policy);
        assert_eq!(0.3, w.gov.player_policy.unwrap().income_tax);
    }
    #[test]
    fn queue_rejects_duplicates_and_reserves_treasury() {
        let mut w = world();
        let existing = w.ledger.balance(w.gov.account);
        if existing > 150_000 {
            w.ledger.burn(w.gov.account, existing - 150_000, w.day);
        } else {
            w.ledger.mint(w.gov.account, 150_000 - existing);
        }
        let first = Command::Build {
            x: 0,
            y: 0,
            zone: Zone::Residential,
        };
        w.apply_command(first.clone()).unwrap();
        assert!(w.apply_command(first).is_err());
        // The first order reserves the entire treasury before spending begins.
        let before = w.gov.pending_builds.clone();
        assert!(w
            .apply_command(Command::Build {
                x: 1,
                y: 0,
                zone: Zone::Park
            })
            .is_err());
        assert_eq!(before, w.gov.pending_builds);
        w.apply_command(Command::CancelBuild { x: 0, y: 0 })
            .unwrap();
        assert!(!w
            .gov
            .pending_builds
            .iter()
            .any(|&(x, y, _)| (x, y) == (0, 0)));
    }
    #[test]
    fn preview_is_read_only_and_matches_submission() {
        let mut w = world();
        w.ledger.mint(w.gov.account, 150_000);
        let cash = w.ledger.balance(w.gov.account);
        assert!(w.preview_build(0, 0, Zone::Residential).is_ok());
        assert!(w.preview_build(usize::MAX, 0, Zone::Residential).is_err());
        assert_eq!(cash, w.ledger.balance(w.gov.account));
        assert!(w.gov.pending_builds.is_empty());
        assert!(w.construction.is_empty());
        w.apply_command(Command::Build {
            x: 0,
            y: 0,
            zone: Zone::Residential,
        })
        .unwrap();
        assert_eq!(
            w.preview_build(0, 0, Zone::Residential).unwrap_err(),
            w.apply_command(Command::Build {
                x: 0,
                y: 0,
                zone: Zone::Residential
            })
            .unwrap_err()
        );
    }
    #[test]
    fn active_cancellation_refunds_only_unspent_escrow() {
        let mut w = world();
        w.ledger.mint(w.gov.account, 150_000);
        let cash = w.ledger.balance(w.gov.account);
        w.start_construction(0, 0, Zone::Residential).unwrap();
        let escrow = w.construction[0].escrow;
        // A delivered input is a sunk cost paid to its supplier.
        w.ledger.transfer(escrow, w.firms.account[0], 100, w.day);
        w.construction[0].spent = 100;
        let result = w
            .apply_command(Command::CancelBuild { x: 0, y: 0 })
            .unwrap();
        assert!(result.message.contains("refunded 149900 cents"));
        assert!(result.message.contains("cannot be recovered"));
        assert!(w.construction.is_empty());
        assert_eq!(cash - 100, w.ledger.balance(w.gov.account));
        assert_eq!(0, w.ledger.balance(escrow));
        w.ledger.assert_conserved(w.day);
        assert!(w
            .apply_command(Command::CancelBuild { x: 0, y: 0 })
            .is_err());
    }
    #[test]
    fn clock_is_independent_of_frame_slicing() {
        let mut whole = SimulationClock::new(100);
        let mut sliced = SimulationClock::new(100);
        let expected = whole.advance(Duration::from_secs(10), 7.0, false, 0);
        let actual: u32 = (0..1000)
            .map(|_| sliced.advance(Duration::from_millis(10), 7.0, false, 0))
            .sum();
        assert_eq!(expected, actual);
        assert_eq!(70, actual);
    }
    #[test]
    fn clock_preserves_debt_and_manual_steps_across_pause() {
        let mut clock = SimulationClock::new(3);
        assert_eq!(3, clock.advance(Duration::from_secs(1), 7.5, false, 0));
        assert_eq!(0, clock.advance(Duration::from_secs(100), 7.5, true, 0));
        assert_eq!(3, clock.advance(Duration::ZERO, 7.5, true, 4));
        assert_eq!(1, clock.advance(Duration::ZERO, 7.5, true, 0));
        assert_eq!(3, clock.advance(Duration::ZERO, 7.5, false, 0));
        assert_eq!(1, clock.advance(Duration::ZERO, 7.5, false, 0));
        assert_eq!(1, clock.advance(Duration::from_millis(200), 2.5, false, 0));
    }
}
