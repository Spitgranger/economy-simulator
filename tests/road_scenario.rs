//! Campaign guardrails for the starter layout, not a claim of economic calibration.
use econsim::{city::{Zone, NO_TILE}, Config, World};
fn output(name: &str) -> String {
    format!("/tmp/econsim-road-scenario-{}-{name}", std::process::id())
}
#[test]
fn default_roads_do_not_trigger_startup_collapse() {
    let dir = output("default");
    let mut w = World::new(Config { out_dir: dir.clone(), quiet: true, ..Config::default() }).unwrap();
    assert_eq!((w.city.w,w.city.h),(48,32));
    assert_eq!(w.city.capacity_of(Zone::Residential),1200);
    assert_eq!(w.city.capacity_of(Zone::Business),120);
    assert!((0..w.homes.n).filter(|&h|w.homes.active[h]).all(|h| w.homes.tile[h] != NO_TILE));
    for _ in 0..20*336 { w.tick_day(); }
    let rows = &w.stats.months;
    assert!(rows[..12].iter().all(|m|m.unemployment < 0.35), "starter congestion must not cause mass unemployment in year one");
    assert!(rows.iter().all(|m|m.unemployment < 0.60), "sustained scenario must avoid the original 70% collapse");
    assert!(rows[rows.len()-12..].iter().map(|m|m.unemployment).sum::<f64>()/12.0 < 0.30);
    assert!(rows.iter().any(|m|m.traffic_peak_flow > 400), "roads still experience congestion");
    drop(w); let _ = std::fs::remove_dir_all(dir);
}
#[test]
fn expanded_scenario_seats_actual_agents_on_road_frontage() {
    let dir = output("large");
    let w = World::new(Config { n_hh:5000,n_firms:500,firm_cap:2000,out_dir:dir.clone(),quiet:true,..Config::default() }).unwrap();
    for h in (0..w.homes.n).filter(|&h|w.homes.active[h]) {
        assert_ne!(w.homes.tile[h],NO_TILE);
        assert!(w.city.road_access(w.homes.tile[h]));
    }
    for &f in &w.firms.active_list { assert!(w.city.road_access(w.firms.tile[f as usize])); }
    drop(w); let _ = std::fs::remove_dir_all(dir);
}
