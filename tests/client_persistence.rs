//! Exercise persistence through the actual CLI, including overwrite/replay ordering.
use econsim::sim::World;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "econsim-cli-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().into()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_econsim"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "CLI failed: {args:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn load(path: &str, out: &str) -> World {
    World::load(Path::new(path), Path::new(out)).unwrap()
}

#[test]
fn resume_duration_and_same_source_save_replay_match_uninterrupted() {
    let scratch = Scratch::new();
    let save = scratch.path("world.save");
    run(&[
        "--seed",
        "917",
        "--households",
        "40",
        "--firms",
        "10",
        "--days",
        "35",
        "--quiet",
        "--out",
        &scratch.path("initial"),
        "--save",
        &save,
    ]);
    let initial = load(&save, &scratch.path("inspect-initial"));
    assert_eq!(initial.day, 35);
    drop(initial);
    let output = run(&[
        "--load",
        &save,
        "--save",
        &save,
        "--days",
        "5",
        "--quiet",
        "--out",
        &scratch.path("resumed"),
        "--replay-check",
    ]);
    assert!(output.contains("replay check: identical"));
    let resumed = load(&save, &scratch.path("inspect-resumed"));
    assert_eq!(resumed.day, 40);
    assert_eq!(resumed.cfg.seed, 917);
    assert_eq!(resumed.cfg.n_hh, 40);
    let uninterrupted_save = scratch.path("uninterrupted.save");
    run(&[
        "--seed",
        "917",
        "--households",
        "40",
        "--firms",
        "10",
        "--days",
        "40",
        "--quiet",
        "--out",
        &scratch.path("uninterrupted"),
        "--save",
        &uninterrupted_save,
    ]);
    let uninterrupted = load(&uninterrupted_save, &scratch.path("inspect-uninterrupted"));
    assert_eq!(resumed.stats.hash(), uninterrupted.stats.hash());
    assert_eq!(resumed.ledger.total(), uninterrupted.ledger.total());

    // A zero-duration continuation of a world with monthly history must save
    // without advancing or discarding that history.
    let zero_save = scratch.path("zero.save");
    run(&[
        "--load",
        &save,
        "--save",
        &zero_save,
        "--days",
        "0",
        "--quiet",
        "--out",
        &scratch.path("zero"),
    ]);
    let zero = load(&zero_save, &scratch.path("inspect-zero"));
    assert_eq!(zero.day, 40);
    assert_eq!(zero.stats.hash(), resumed.stats.hash());
    assert_eq!(zero.stats.months.len(), resumed.stats.months.len());
}

#[test]
fn short_run_save_and_replay_can_resume_before_first_month() {
    let scratch = Scratch::new();
    let save = scratch.path("short.save");
    run(&[
        "--seed",
        "23",
        "--households",
        "30",
        "--firms",
        "8",
        "--days",
        "3",
        "--quiet",
        "--out",
        &scratch.path("initial"),
        "--save",
        &save,
        "--replay-check",
    ]);
    let initial = load(&save, &scratch.path("inspect-initial"));
    assert_eq!(initial.day, 3);
    assert!(initial.stats.months.is_empty());
    let output = run(&[
        "--load",
        &save,
        "--save",
        &save,
        "--days",
        "2",
        "--quiet",
        "--out",
        &scratch.path("resumed"),
        "--replay-check",
    ]);
    assert!(output.contains("replay check: deterministic"));
    let resumed = load(&save, &scratch.path("inspect-resumed"));
    assert_eq!(resumed.day, 5);
    assert!(resumed.stats.months.is_empty());
    assert_ne!(resumed.stats.hash(), initial.stats.hash());
}
