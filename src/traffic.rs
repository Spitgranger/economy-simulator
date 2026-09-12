//! Deterministic road assignment. Routes remember month-boundary congestion;
//! their daily costs use the preceding day's flows. Trips count real decisions,
//! including off-network walking; freight requires a connected road route.
use crate::{
    city::{Tile, Zone, NO_TILE},
    sim::World,
};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
pub const ROAD_CAPACITY: f64 = 400.0;
pub fn cross_cost(flow: u64) -> f64 {
    1.0 + 0.15 * (flow as f64 / ROAD_CAPACITY).powi(4)
}
pub fn neighbors(w: usize, n: usize, t: u16) -> impl Iterator<Item = u16> {
    let i = t as usize;
    [
        if i >= w { Some((i - w) as u16) } else { None },
        if i % w > 0 {
            Some((i - 1) as u16)
        } else {
            None
        },
        if i % w + 1 < w {
            Some((i + 1) as u16)
        } else {
            None
        },
        if i + w < n {
            Some((i + w) as u16)
        } else {
            None
        },
    ]
    .into_iter()
    .flatten()
}
pub fn entries(tiles: &[Tile], w: usize, t: u16) -> impl Iterator<Item = u16> + '_ {
    let valid = (t as usize) < tiles.len();
    let own = if valid && tiles[t as usize].zone == Zone::Road {
        Some(t)
    } else {
        None
    };
    own.into_iter().chain(
        neighbors(w, tiles.len(), if valid { t } else { 0 })
            .filter(move |&i| valid && own.is_none() && tiles[i as usize].zone == Zone::Road),
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TripKind {
    Commute = 0,
    Shopping = 1,
    Freight = 2,
}
#[derive(Clone, Default, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TripCounts {
    pub requested: [u64; 3],
    pub routed: [u64; 3],
    pub off_network: [u64; 3],
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Source {
    distance: Vec<f64>,
    parent: Vec<u16>,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Route {
    pair: (u16, u16),
    tiles: Vec<u16>,
    cost: f64,
    counts: [u64; 3],
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Trip {
    pub pair: (u16, u16),
    pub counts: [u64; 3],
}
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Traffic {
    pub flows: Vec<u64>,
    pub daily: TripCounts,
    pub monthly: TripCounts,
    pub monthly_peak_flow: u64,
    pub monthly_delay_sum: f64,
    pub monthly_days: u32,
    pub trips: Vec<Trip>,
    pub(crate) shopping_pairs: Vec<(u32, u32)>,
    #[serde(skip)]
    sources: BTreeMap<u16, Source>,
    routes: Vec<Route>,
    #[serde(skip)]
    pair_index: Vec<u32>,
    pending_trips: Vec<Trip>,
    active_trips: Vec<usize>,
    // Frozen assignment weights; current daily route costs are separate.
    weights: Vec<f64>,
    dirty: bool,
}
#[derive(Clone, Copy, PartialEq)]
struct Node(f64, u16);
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        o.0.total_cmp(&self.0).then_with(|| o.1.cmp(&self.1))
    }
}
impl Traffic {
    pub fn new(n: usize) -> Self {
        Self {
            flows: vec![0; n],
            daily: TripCounts::default(),
            monthly: TripCounts::default(),
            monthly_peak_flow: 0,
            monthly_delay_sum: 0.0,
            monthly_days: 0,
            trips: Vec::new(),
            shopping_pairs: Vec::new(),
            sources: BTreeMap::new(),
            routes: Vec::new(),
            pair_index: vec![u32::MAX; n * n],
            active_trips: Vec::new(),
            pending_trips: Vec::new(),
            weights: vec![1.0; n],
            dirty: false,
        }
    }
    /// Restore only lookup accelerators; frozen weights and chosen paths survive.
    pub(crate) fn rebuild_caches(&mut self) {
        self.sources.clear();
        let n = self.flows.len();
        self.pair_index = vec![u32::MAX; n * n];
        for (i, route) in self.routes.iter().enumerate() {
            self.pair_index[route.pair.0 as usize * n + route.pair.1 as usize] = i as u32;
        }
    }
    pub fn invalidate(&mut self) {
        for &i in &self.active_trips {
            self.pending_trips.push(Trip {
                pair: self.routes[i].pair,
                counts: self.routes[i].counts,
            });
        }
        self.sources.clear();
        self.routes.clear();
        self.pair_index.fill(u32::MAX);
        self.active_trips.clear();
        self.dirty = true;
    }
    pub fn begin_day(&mut self, day: u32) {
        if day % 28 == 1 || self.dirty {
            self.invalidate();
            self.weights = self.flows.iter().map(|&f| cross_cost(f)).collect();
            self.dirty = false;
        }
        for route in self.routes.iter_mut() {
            route.cost = route
                .tiles
                .iter()
                .map(|&t| cross_cost(self.flows[t as usize]))
                .sum();
        }
        self.pending_trips.clear();
        self.trips.clear();
        for &i in &self.active_trips {
            self.routes[i].counts = [0; 3];
        }
        self.active_trips.clear();
        self.shopping_pairs.clear();
        self.daily = TripCounts::default();
    }
    pub fn ensure_route(&mut self, tiles: &[Tile], w: usize, a: u16, b: u16) -> bool {
        if a == NO_TILE || b == NO_TILE || a as usize >= tiles.len() || b as usize >= tiles.len() {
            return false;
        }
        let pair = a as usize * tiles.len() + b as usize;
        let cached = self.pair_index[pair];
        if cached != u32::MAX {
            return !self.routes[cached as usize].tiles.is_empty();
        }
        if self.dirty {
            self.weights = self.flows.iter().map(|&f| cross_cost(f)).collect();
            self.dirty = false;
        }
        if !self.sources.contains_key(&a) {
            let mut source = Source {
                distance: vec![f64::INFINITY; tiles.len()],
                parent: vec![NO_TILE; tiles.len()],
            };
            let mut heap = BinaryHeap::new();
            for t in entries(tiles, w, a) {
                let cost = self.weights[t as usize];
                source.distance[t as usize] = cost;
                heap.push(Node(cost, t));
            }
            while let Some(Node(cost, t)) = heap.pop() {
                if cost > source.distance[t as usize] {
                    continue;
                }
                for next in
                    neighbors(w, tiles.len(), t).filter(|&t| tiles[t as usize].zone == Zone::Road)
                {
                    let nc = cost + self.weights[next as usize];
                    if nc < source.distance[next as usize] {
                        source.distance[next as usize] = nc;
                        source.parent[next as usize] = t;
                        heap.push(Node(nc, next));
                    }
                }
            }
            self.sources.insert(a, source);
        }
        let source = &self.sources[&a];
        let end = entries(tiles, w, b)
            .filter(|&t| source.distance[t as usize].is_finite())
            .min_by(|&x, &y| {
                source.distance[x as usize]
                    .total_cmp(&source.distance[y as usize])
                    .then(x.cmp(&y))
            });
        let mut path = Vec::new();
        if let Some(mut t) = end {
            loop {
                path.push(t);
                let p = source.parent[t as usize];
                if p == NO_TILE {
                    break;
                }
                t = p;
            }
            path.reverse();
        }
        let routed = !path.is_empty();
        let cost = path
            .iter()
            .map(|&t| cross_cost(self.flows[t as usize]))
            .sum();
        self.pair_index[pair] = self.routes.len() as u32;
        self.routes.push(Route {
            pair: (a, b),
            tiles: path,
            cost,
            counts: [0; 3],
        });
        routed
    }
    fn route(&self, a: u16, b: u16) -> Option<&Route> {
        if a as usize >= self.flows.len() || b as usize >= self.flows.len() {
            return None;
        }
        let i = self.pair_index[a as usize * self.flows.len() + b as usize];
        if i == u32::MAX {
            None
        } else {
            Some(&self.routes[i as usize])
        }
    }
    pub fn route_cost(&self, a: u16, b: u16) -> Option<f64> {
        self.route(a, b)
            .filter(|r| !r.tiles.is_empty())
            .map(|r| r.cost)
    }
    pub fn add_trip(&mut self, a: u16, b: u16, kind: TripKind, count: u64) {
        if count == 0 || a as usize >= self.flows.len() || b as usize >= self.flows.len() {
            return;
        }
        let i = self.pair_index[a as usize * self.flows.len() + b as usize];
        if i == u32::MAX {
            self.pending_trips.push(Trip {
                pair: (a, b),
                counts: std::array::from_fn(|k| if k == kind as usize { count } else { 0 }),
            });
            return;
        }
        let i = i as usize;
        if self.routes[i].counts == [0; 3] {
            self.active_trips.push(i);
        }
        self.routes[i].counts[kind as usize] += count;
    }
    pub fn finish_day(&mut self) {
        self.flows.fill(0);
        for &i in &self.active_trips {
            let route = &self.routes[i];
            let counts = &route.counts;
            let routed = !route.tiles.is_empty();
            for k in 0..3 {
                self.daily.requested[k] += counts[k];
                if routed {
                    self.daily.routed[k] += counts[k];
                } else {
                    self.daily.off_network[k] += counts[k];
                }
            }
            let total = counts.iter().sum::<u64>();
            for &t in &route.tiles {
                self.flows[t as usize] += total;
            }
        }
        self.trips = self
            .active_trips
            .iter()
            .map(|&i| Trip {
                pair: self.routes[i].pair,
                counts: self.routes[i].counts,
            })
            .collect();
        // Uncached demand is off-network unless the world assigned it first.
        for trip in self.pending_trips.drain(..) {
            for k in 0..3 {
                self.daily.requested[k] += trip.counts[k];
                self.daily.off_network[k] += trip.counts[k];
            }
            self.trips.push(trip);
        }
        self.trips.sort_unstable_by_key(|t| t.pair);
        let mut merged: Vec<Trip> = Vec::with_capacity(self.trips.len());
        for trip in self.trips.drain(..) {
            if let Some(last) = merged.last_mut().filter(|last| last.pair == trip.pair) {
                for k in 0..3 {
                    last.counts[k] += trip.counts[k];
                }
            } else {
                merged.push(trip);
            }
        }
        self.trips = merged;
        for i in self.active_trips.drain(..) {
            self.routes[i].counts = [0; 3];
        }
        for k in 0..3 {
            self.monthly.requested[k] += self.daily.requested[k];
            self.monthly.routed[k] += self.daily.routed[k];
            self.monthly.off_network[k] += self.daily.off_network[k];
        }
        self.monthly_peak_flow = self
            .monthly_peak_flow
            .max(self.flows.iter().copied().max().unwrap_or(0));
        let used = self.flows.iter().filter(|&&f| f > 0).count();
        self.monthly_delay_sum += if used == 0 {
            1.0
        } else {
            self.flows
                .iter()
                .filter(|&&f| f > 0)
                .map(|&f| cross_cost(f))
                .sum::<f64>()
                / used as f64
        };
        self.monthly_days += 1;
    }
    pub fn reset_month(&mut self) {
        self.monthly = TripCounts::default();
        self.monthly_peak_flow = 0;
        self.monthly_delay_sum = 0.0;
        self.monthly_days = 0;
    }
}
impl World {
    pub(crate) fn traffic_begin_day(&mut self) {
        self.city.traffic.begin_day(self.day);
        // Cache all current home/work tile combinations, including prospective
        // employers used by the weekly labor market. This is bounded by map size.
        let origins: BTreeSet<_> = self
            .homes
            .tile
            .iter()
            .copied()
            .filter(|&t| t != NO_TILE)
            .collect();
        let destinations: BTreeSet<_> = self
            .firms
            .active_list
            .iter()
            .map(|&f| self.firms.tile[f as usize])
            .filter(|&t| t != NO_TILE)
            .collect();
        for a in origins {
            for &b in &destinations {
                self.city.ensure_route(a, b);
            }
        }
        self.firms.effective_labor.fill(0.0);
        for i in 0..self.ppl.n {
            if !self.ppl.alive[i] || !self.ppl.is_employed(i) {
                continue;
            }
            let f = self.ppl.employer[i] as usize;
            if f >= self.firms.active.len() || !self.firms.active[f] {
                continue;
            }
            self.firms.effective_labor[f] += self.labor_of(i, f);
            let home = self.ppl.home[i];
            if home == crate::people::NO_HOME {
                continue;
            }
            let a = self.homes.tile[home as usize];
            let b = self.firms.tile[f];
            self.city.ensure_route(b, a);
            self.city.traffic.add_trip(a, b, TripKind::Commute, 1);
            self.city.traffic.add_trip(b, a, TripKind::Commute, 1);
        }
    }
    pub(crate) fn traffic_finish_day(&mut self) {
        let mut purchases = std::mem::take(&mut self.city.traffic.shopping_pairs);
        purchases.sort_unstable();
        purchases.dedup();
        for (h, f) in purchases {
            let a = self.homes.tile[h as usize];
            let b = self.firms.tile[f as usize];
            self.city.ensure_route(a, b);
            self.city.traffic.add_trip(a, b, TripKind::Shopping, 1);
        }
        // Construction may have completed a road and invalidated earlier paths.
        let detached = std::mem::take(&mut self.city.traffic.pending_trips);
        for t in detached {
            let (a, b) = t.pair;
            self.city.ensure_route(a, b);
            for (k, kind) in [TripKind::Commute, TripKind::Shopping, TripKind::Freight]
                .iter()
                .enumerate()
            {
                if t.counts[k] > 0 {
                    self.city.traffic.add_trip(a, b, *kind, t.counts[k]);
                }
            }
        }
        self.city.traffic.finish_day();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{city::City, sim::Config};
    fn city(w: usize, h: usize) -> City {
        let mut c = City::starter(w, h);
        for t in &mut c.tiles {
            t.zone = Zone::Empty;
            t.occupants = 0;
        }
        c.traffic = Traffic::new(w * h);
        c
    }
    fn world() -> World {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let mut cfg = Config::default();
        cfg.n_hh = 40;
        cfg.n_firms = 10;
        cfg.quiet = true;
        cfg.out_dir = format!(
            "/tmp/econsim-traffic-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        World::new(cfg).unwrap()
    }
    #[test]
    fn topology_changes_preserve_each_trip_once_and_do_not_replay_yesterday() {
        let mut w = world();
        w.city = city(7, 5);
        for x in 1..6 {
            w.city.set(x, 2, Zone::Road);
        }
        let (a, b) = (w.city.idx(0, 2), w.city.idx(6, 2));
        w.city.traffic.begin_day(1);
        w.city.ensure_route(a, b);
        w.city.traffic.add_trip(a, b, TripKind::Commute, 10);
        w.city.set(3, 2, Zone::Empty);
        w.city.ensure_route(a, b);
        w.city.traffic.add_trip(a, b, TripKind::Shopping, 2);
        w.city.set(3, 2, Zone::Road);
        w.traffic_finish_day();
        assert_eq!(w.city.traffic.daily.requested, [10, 2, 0]);
        assert_eq!(w.city.traffic.daily.routed, [10, 2, 0]);
        assert_eq!(w.city.traffic.trips.len(), 1);
        assert_eq!(w.city.traffic.flows[w.city.idx(3, 2) as usize], 12);
        // Changing roads between ticks cannot detach yesterday's completed trips.
        w.city.set(3, 2, Zone::Empty);
        assert!(w.city.traffic.pending_trips.is_empty());
        w.city.traffic.begin_day(2);
        w.traffic_finish_day();
        assert_eq!(w.city.traffic.daily.requested, [0; 3]);
        assert_eq!(w.city.traffic.monthly.requested, [10, 2, 0]);
        assert!(w.city.traffic.flows.iter().all(|&n| n == 0));
        // Uncached demand must also appear in accounting instead of disappearing.
        w.city.traffic.begin_day(3);
        w.city.traffic.add_trip(a, b, TripKind::Shopping, 3);
        w.city.traffic.finish_day();
        assert_eq!(w.city.traffic.daily.off_network, [0, 3, 0]);
    }

    #[test]
    fn road_commands_queue_fund_complete_and_demolish() {
        use crate::commands::Command;
        let mut w = world();
        w.ledger.mint(w.gov.account, 1_000_000);
        w.gov.policy.surplus_dividend = 0.0;
        // A road spur on the existing x=3 corridor, with an isolated endpoint.
        w.city.set(2, 0, Zone::Empty);
        w.city.set(1, 0, Zone::Empty);
        let a = w.city.idx(3, 0);
        let b = w.city.idx(1, 0);
        w.apply_command(Command::Build {
            x: 2,
            y: 0,
            zone: Zone::Road,
        })
        .unwrap();
        assert_eq!(w.gov.pending_builds.len(), 1);
        assert!(!w.city.ensure_route(a, b));
        while w.day < 28 {
            w.tick_day();
        }
        assert!(w.gov.pending_builds.is_empty());
        assert_eq!(w.construction.len(), 1);
        assert!(!w.city.ensure_route(a, b));
        for _ in 0..4 {
            for &f in &w.firms.active_list {
                let f = f as usize;
                if w.firms.good[f] as usize == crate::goods::SHELTER {
                    w.firms.inventory[f] = 1000;
                    w.firms.effective_labor[f] = 100.0;
                }
            }
            w.construction_day();
        }
        assert!(w.construction.is_empty());
        assert_eq!(w.city.tiles[w.city.idx(2, 0) as usize].zone, Zone::Road);
        assert!(w.city.ensure_route(a, b));
        w.apply_command(Command::Build {
            x: 2,
            y: 0,
            zone: Zone::Empty,
        })
        .unwrap();
        assert!(w.city.ensure_route(a, b));
        while w.day < 56 {
            w.tick_day();
        }
        assert_eq!(w.city.tiles[w.city.idx(2, 0) as usize].zone, Zone::Empty);
        assert!(!w.city.ensure_route(a, b));
        w.ledger.assert_conserved(w.day);
    }

    #[test]
    fn road_endpoints_are_part_of_the_route() {
        let mut c = city(5, 3);
        for x in 0..5 { c.set(x, 1, Zone::Road); }
        let (a,b) = (c.idx(0,1), c.idx(4,1));
        assert!(c.ensure_route(a,b));
        assert_eq!(c.traffic.route(a,b).unwrap().tiles, (5..10).collect::<Vec<u16>>());
        assert_eq!(c.route_cost(a,b), 5.0);
    }

    #[test]
    fn orthogonal_routes_walking_ties_and_demolition() {
        let mut c = city(7, 7);
        // Two equal length paths; priority order chooses upper indexed path.
        for x in 1..6 {
            c.set(x, 1, Zone::Road);
            c.set(x, 3, Zone::Road);
        }
        c.set(1, 2, Zone::Road);
        c.set(5, 2, Zone::Road);
        let (a, b) = (c.idx(0, 2), c.idx(6, 2));
        assert!(c.ensure_route(a, b));
        let route = &c.traffic.route(a, b).unwrap().tiles;
        assert!(route.contains(&c.idx(3, 1)));
        assert!(!route.contains(&c.idx(3, 3)));
        assert!(c.route_cost(a, b) < c.distance(a, b) as f64 + 8.0);
        c.set(3, 1, Zone::Empty);
        assert!(c.ensure_route(a, b));
        assert!(c.traffic.route(a, b).unwrap().tiles.contains(&c.idx(3, 3)));
        c.set(3, 3, Zone::Empty);
        assert!(!c.ensure_route(a, b));
        assert_eq!(c.route_cost(a, b), 14.0);
        let diagonal = c.idx(0, 0);
        assert!(!c.road_access(diagonal));
    }
    #[test]
    fn congestion_lags_one_day_and_parallel_route_restores_labor() {
        let mut c = city(9, 7);
        for x in 1..8 {
            c.set(x, 2, Zone::Road);
        }
        let (a, b) = (c.idx(0, 2), c.idx(8, 2));
        c.traffic.begin_day(1);
        assert!(c.ensure_route(a, b));
        let clear = c.work_factor(a, b);
        c.traffic.add_trip(a, b, TripKind::Commute, 1200);
        c.traffic.finish_day();
        assert_eq!(
            c.work_factor(a, b),
            clear,
            "today's trips cannot change today's labor"
        );
        c.traffic.begin_day(2);
        let jammed = c.work_factor(a, b);
        assert!(jammed < clear);
        // A parallel corridor with separate access avoids the overloaded cells.
        for x in 0..9 {
            c.set(x, 1, Zone::Road);
        }
        assert!(c.ensure_route(a, b));
        assert!(c.work_factor(a, b) > jammed);
        let output = |factor: f64| (20.0 * factor * 3.0).floor() as i64;
        assert!(output(c.work_factor(a, b)) > output(jammed));
    }
    #[test]
    fn congested_commutes_reduce_real_production_and_new_corridor_recovers_it() {
        let mut w = world();
        w.city = city(9, 7);
        for x in 1..8 {
            w.city.set(x, 2, Zone::Road);
        }
        let (a, b) = (w.city.idx(0, 2), w.city.idx(8, 2));
        w.homes.tile.fill(a);
        for &f in &w.firms.active_list {
            w.firms.tile[f as usize] = b;
        }
        // Run ordinary engine days so production, purchases, and both physical
        // and monetary conservation execute, not a duplicated output formula.
        w.tick_day();
        let clear_labor: f64 = w.firms.effective_labor.iter().sum();
        let clear_output: i128 = w.physical_goods_balance().produced.iter().sum();
        w.city.traffic.flows.fill(0);
        for x in 1..8 {
            let t = w.city.idx(x, 2);
            w.city.traffic.flows[t as usize] = 1200;
        }
        w.tick_day();
        let jammed_labor: f64 = w.firms.effective_labor.iter().sum();
        let jammed_output: i128 = w.physical_goods_balance().produced.iter().sum();
        assert!(jammed_labor < clear_labor);
        assert!(jammed_output < clear_output);
        // Keep the old corridor jammed while topology opens a separate route.
        for x in 1..8 {
            let t = w.city.idx(x, 2);
            w.city.traffic.flows[t as usize] = 1200;
        }
        for x in 0..9 {
            w.city.set(x, 1, Zone::Road);
        }
        w.tick_day();
        assert!(w.firms.effective_labor.iter().sum::<f64>() > jammed_labor);
        assert!(w.physical_goods_balance().produced.iter().sum::<i128>() > jammed_output);
    }

    #[test]
    fn agent_commutes_follow_layoffs_and_purchases_are_distinct() {
        let mut w = world();
        let mut employed: Vec<_> = (0..w.ppl.n).filter(|&i| w.ppl.is_employed(i)).collect();
        if employed.len() % 2 == 1 {
            let i = employed.pop().unwrap();
            w.ppl.employer[i] = crate::people::NO_FIRM;
        }
        assert!(!employed.is_empty());
        w.traffic_begin_day();
        w.traffic_finish_day();
        let total = w.city.traffic.daily.requested[0];
        assert_eq!(total, employed.len() as u64 * 2);
        for &i in &employed[..employed.len() / 2] {
            w.ppl.employer[i] = crate::people::NO_FIRM;
        }
        w.traffic_begin_day();
        w.city.traffic.shopping_pairs.push((0, 0));
        w.city.traffic.shopping_pairs.push((0, 0));
        w.traffic_finish_day();
        assert_eq!(w.city.traffic.daily.requested[0], total / 2);
        assert_eq!(w.city.traffic.daily.requested[1], 1);
        assert_eq!(
            w.city.traffic.daily.routed[0] + w.city.traffic.daily.off_network[0],
            total / 2
        );
    }
    #[test]
    fn disconnected_freight_stalls_then_road_extension_delivers() {
        let mut w = world();
        w.ledger.mint(w.gov.account, 1_000_000);
        for &f in &w.firms.active_list {
            let f = f as usize;
            if w.firms.good[f] as usize == crate::goods::SHELTER {
                w.firms.inventory[f] = 1000;
                w.firms.effective_labor[f] = 100.0;
            }
        }
        w.start_construction(0, 0, Zone::Residential).unwrap();
        w.construction_day();
        assert_eq!(
            w.construction[0].stall,
            crate::construction::ConstructionStall::Access
        );
        assert_eq!(w.construction[0].materials_delivered, 0);
        // Starter road x=3; extending through (2,0),(1,0) reaches the site.
        w.city.set(2, 0, Zone::Empty);
        w.city.set(1, 0, Zone::Empty);
        w.start_construction(2, 0, Zone::Road).unwrap();
        assert!(!w.city.ensure_route(w.firms.tile[0], w.city.idx(0, 0)));
        for _ in 0..4 {
            w.construction_day();
        }
        assert_eq!(w.city.tiles[2].zone, Zone::Road);
        w.start_construction(1, 0, Zone::Road).unwrap();
        for _ in 0..4 {
            w.construction_day();
        }
        w.construction_day();
        assert!(
            w.construction
                .iter()
                .find(|p| p.tile == 0)
                .unwrap()
                .materials_delivered
                > 0
        );
        assert!(w.city.traffic.routes.iter().any(|r| r.counts[2] > 0));
        w.ledger.assert_conserved(w.day);
    }
    #[test]
    fn larger_starter_seats_five_hundred_firms_and_five_thousand_homes() {
        let c = City::starter(48, 32);
        assert!(c.capacity_of(Zone::Business) >= 500);
        assert!(c.capacity_of(Zone::Residential) >= 5000);
        for (i, t) in c.tiles.iter().enumerate() {
            if t.zone.capacity() > 0 {
                assert!(c.road_access(i as u16));
            }
        }
    }
}
