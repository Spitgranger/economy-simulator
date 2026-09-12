//! The city: a grid of tiles the player zones. Homes need residential room,
//! firms need business room, pupils need school room. Land value follows
//! occupancy and neighbours; homes and firms pay ground rent on it to the
//! treasury. Commuting distance lowers a worker's effective output.

pub const NO_TILE: u16 = u16::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
#[derive(serde::Serialize, serde::Deserialize)]
pub enum Zone {
    Empty = 0,
    Residential = 1,
    Business = 2,
    School = 3,
    Park = 4,
    Road = 5,
}

impl Zone {
    pub fn from_u8(v: u8) -> Zone {
        match v {
            1 => Zone::Residential,
            2 => Zone::Business,
            3 => Zone::School,
            4 => Zone::Park,
            5 => Zone::Road,
            _ => Zone::Empty,
        }
    }
    /// Homes, firms or pupils a tile of this zone can hold.
    pub fn capacity(self) -> u16 {
        match self {
            Zone::Residential => 20,
            Zone::Business => 6,
            Zone::School => 250,
            _ => 0,
        }
    }
    /// Construction cost in cents, paid to the building trade.
    pub fn cost(self) -> i64 {
        match self {
            Zone::Residential => 150_000,
            Zone::Business => 250_000,
            Zone::School => 400_000,
            Zone::Park => 80_000,
            Zone::Road => 60_000,
            Zone::Empty => 0,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Zone::Empty => "empty",
            Zone::Residential => "residential",
            Zone::Business => "business",
            Zone::School => "school",
            Zone::Park => "park",
            Zone::Road => "road",
        }
    }
}

#[derive(Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Tile {
    pub zone: Zone,
    pub occupants: u16, // homes on residential, firms on business
    pub land_value: i64,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct City {
    pub w: usize,
    pub h: usize,
    pub tiles: Vec<Tile>,
    pub rent_month: i64,
    pub built_month: i64,
    pub traffic: crate::traffic::Traffic,
}

impl City {
    /// A compact starter city: a business core, a residential ring, one school, two parks.
    pub fn starter(w: usize, h: usize) -> City {
        let mut c = City { w, h, tiles: vec![Tile { zone: Zone::Empty, occupants: 0, land_value: 100_000 }; w * h], rent_month: 0, built_month: 0, traffic: crate::traffic::Traffic::new(w*h) };
        // Streets at 3 mod 7 leave (0,0) vacant for expansion. Frontage is
        // occupied first; interiors remain vacant for player development.
        for y in 0..h {
            for x in 0..w {
                let frontage = x % 7 == 2 || x % 7 == 4 || y % 7 == 2 || y % 7 == 4;
                let zone = if x % 7 == 3 || y % 7 == 3 { Zone::Road }
                    else if frontage && (x + 2*y) % 4 == 0 { Zone::Business }
                    else if frontage { Zone::Residential } else { Zone::Empty };
                c.tiles[y*w+x].zone = zone;
            }
        }
        let school = (0..c.tiles.len()).filter(|&i| c.tiles[i].zone == Zone::Residential)
            .min_by_key(|&i| c.distance(i as u16, c.idx(w/2,h/2)));
        if let Some(i) = school { c.tiles[i].zone = Zone::School; }
        c.set(1, 2, Zone::Park);
        c.update_land_values();
        c
    }

    /// Zone only enough central frontage for the scenario plus 20% headroom.
    /// Roads span the region; empty frontage remains available for expansion.
    pub fn size_starter_neighborhood(&mut self, homes: usize, firms: usize) {
        let center = self.idx(self.w / 2, self.h / 2);
        for (zone, count) in [(Zone::Residential, homes), (Zone::Business, firms)] {
            let cap = zone.capacity() as usize;
            let needed = (count.saturating_add(count / 5).saturating_add(cap - 1) / cap).max(1);
            let mut lots: Vec<_> = self.tiles.iter().enumerate()
                .filter(|(_, tile)| tile.zone == zone).map(|(i, _)| i as u16).collect();
            lots.sort_unstable_by_key(|&t| (self.distance(center, t), t));
            for &t in lots.iter().skip(needed) { self.tiles[t as usize].zone = Zone::Empty; }
        }
        self.update_land_values();
    }

    #[inline]
    pub fn idx(&self, x: usize, y: usize) -> u16 {
        (y * self.w + x) as u16
    }

    #[inline]
    pub fn xy(&self, t: u16) -> (i32, i32) {
        ((t as usize % self.w) as i32, (t as usize / self.w) as i32)
    }

    pub fn set(&mut self, x: usize, y: usize, zone: Zone) {
        if x < self.w && y < self.h {
            if (self.tiles[y * self.w + x].zone == Zone::Road) != (zone == Zone::Road) { self.traffic.invalidate(); }
            let t = &mut self.tiles[y * self.w + x];
            t.zone = zone;
            t.occupants = 0;
        }
    }

    /// Manhattan distance; an unhoused party counts as living at the edge.
    pub fn distance(&self, a: u16, b: u16) -> i32 {
        if a == NO_TILE || b == NO_TILE {
            return (self.w + self.h) as i32 / 2;
        }
        let (ax, ay) = self.xy(a);
        let (bx, by) = self.xy(b);
        (ax - bx).abs() + (ay - by).abs()
    }

    /// Share of output after travel. Congested road cost has different units
    /// from the former Manhattan distance; 0.006 is the starter-scenario
    /// calibration, with the existing 40% lower bound retained.
    pub fn work_factor(&self, home: u16, work: u16) -> f64 {
        (1.0 - 0.006 * self.route_cost(home, work)).max(0.4)
    }

    pub fn ensure_route(&mut self, a: u16, b: u16) -> bool {
        self.traffic.ensure_route(&self.tiles, self.w, a, b)
    }
    pub fn route_cost(&self, a: u16, b: u16) -> f64 {
        self.traffic.route_cost(a, b).unwrap_or(self.distance(a,b) as f64 + 8.0)
    }
    pub fn road_access(&self, tile: u16) -> bool {
        crate::traffic::entries(&self.tiles, self.w, tile).next().is_some()
    }

    /// Nearest tile of `zone` with free room to `near` (or the centre).
    pub fn find_free(&self, zone: Zone, near: u16) -> Option<u16> {
        let center = self.idx(self.w / 2, self.h / 2);
        let from = if near == NO_TILE { center } else { near };
        let mut best: Option<(i32, u16)> = None;
        for (i, t) in self.tiles.iter().enumerate() {
            if t.zone == zone && t.occupants < zone.capacity() {
                let d = self.distance(from, i as u16);
                if best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, i as u16));
                }
            }
        }
        best.map(|(_, t)| t)
    }

    pub fn occupy(&mut self, t: u16) {
        if t != NO_TILE {
            self.tiles[t as usize].occupants += 1;
        }
    }

    pub fn vacate(&mut self, t: u16) {
        if t != NO_TILE {
            let tile = &mut self.tiles[t as usize];
            tile.occupants = tile.occupants.saturating_sub(1);
        }
    }

    pub fn capacity_of(&self, zone: Zone) -> u32 {
        self.tiles.iter().filter(|t| t.zone == zone).count() as u32 * zone.capacity() as u32
    }

    pub fn occupants_of(&self, zone: Zone) -> u32 {
        self.tiles.iter().filter(|t| t.zone == zone).map(|t| t.occupants as u32).sum()
    }

    /// Land value: a base, plus occupancy, plus business, park and school
    /// tiles within reach, less distance from the centre.
    pub fn update_land_values(&mut self) {
        let (cx, cy) = (self.w as i32 / 2, self.h as i32 / 2);
        let snapshot: Vec<(Zone, f64)> = self.tiles.iter().map(|t| (t.zone, if t.zone.capacity() > 0 { t.occupants as f64 / t.zone.capacity() as f64 } else { 0.0 })).collect();
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let i = (y as usize) * self.w + x as usize;
                let (zone, occ) = snapshot[i];
                let mut v = 100_000.0 + 60_000.0 * occ;
                for dy in -3..=3i32 {
                    for dx in -3..=3i32 {
                        let (nx, ny) = (x + dx, y + dy);
                        if nx < 0 || ny < 0 || nx >= self.w as i32 || ny >= self.h as i32 || (dx == 0 && dy == 0) {
                            continue;
                        }
                        let d = dx.abs() + dy.abs();
                        let (nz, nocc) = snapshot[(ny as usize) * self.w + nx as usize];
                        match nz {
                            Zone::Road if d == 1 => v += 8_000.0 / crate::traffic::cross_cost(self.traffic.flows[(ny as usize) * self.w + nx as usize]),
                            Zone::Business if d <= 2 => v += 15_000.0 * (0.5 + nocc),
                            Zone::Park if d <= 2 => v += 20_000.0 / d as f64,
                            Zone::School if d <= 3 => v += 8_000.0,
                            Zone::Residential if d <= 2 => v += 4_000.0 * nocc,
                            _ => {}
                        }
                    }
                }
                v -= 4_000.0 * ((x - cx).abs() + (y - cy).abs()) as f64;
                if zone == Zone::Empty {
                    v *= 0.6;
                }
                self.tiles[i].land_value = v.max(20_000.0) as i64;
            }
        }
    }
}
