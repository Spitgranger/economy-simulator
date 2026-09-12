//! Versioned snapshots of the complete engine state, including RNG streams.
//! CSV and event files are continuation logs; in-memory history and hashes survive.
use crate::sim::World;
use bincode::Options;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"ECONSAVE";
const VERSION: u32 = 3;
const MAX_BYTES: u64 = 512 * 1024 * 1024;

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ *byte as u64).wrapping_mul(0x100000001b3)
    })
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

fn codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_BYTES)
        .reject_trailing_bytes()
}

impl World {
    /// Write to a sibling temporary file and rename only after data is synced.
    /// An interrupted write leaves the previous snapshot intact.
    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let bytes = codec().serialize(self).map_err(invalid)?;
        let name = path
            .file_name()
            .ok_or_else(|| invalid("save path needs a filename"))?;
        let mut temp_name = name.to_os_string();
        temp_name.push(format!(".{}.tmp", std::process::id()));
        let temporary = path.with_file_name(temp_name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(MAGIC)?;
            file.write_all(&VERSION.to_le_bytes())?;
            file.write_all(&checksum(&bytes).to_le_bytes())?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    /// Load a trusted local snapshot into a new output directory. Existing
    /// directories are rejected to avoid truncating an earlier run's logs.
    pub fn load(path: impl AsRef<Path>, out_dir: impl AsRef<Path>) -> io::Result<Self> {
        let file = fs::File::open(path)?;
        if file.metadata()?.len() > MAX_BYTES + 20 {
            return Err(invalid("snapshot exceeds size limit"));
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 21).read_to_end(&mut bytes)?;
        if bytes.len() < 12 || &bytes[..8] != MAGIC {
            return Err(invalid("invalid snapshot header"));
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if version != VERSION {
            return Err(invalid(format!(
                "unsupported snapshot version {version}; expected {VERSION}"
            )));
        }
        if bytes.len() < 20 {
            return Err(invalid("truncated snapshot"));
        }
        let expected = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
        if checksum(&bytes[20..]) != expected {
            return Err(invalid("snapshot checksum mismatch"));
        }
        let mut world: Self = codec().deserialize(&bytes[20..]).map_err(invalid)?;
        world.city.traffic.rebuild_caches();
        let out_dir = out_dir.as_ref();
        let output = out_dir
            .to_str()
            .ok_or_else(|| invalid("output path must be UTF-8"))?
            .to_owned();
        fs::create_dir(out_dir)?;
        world.stats.reattach(&output)?;
        world.events.reattach(&format!("{output}/events.log"))?;
        world.watch.reattach(&format!("{output}/watch.log"))?;
        world.cfg.out_dir = output;
        Ok(world)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Config;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn directory() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "econsim-save-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        dir
    }
    #[test]
    fn snapshot_continuation_matches_uninterrupted_state() {
        let dir = directory();
        let cfg = Config {
            n_hh: 60,
            n_firms: 12,
            firm_cap: 24,
            quiet: true,
            out_dir: dir.join("original").to_str().unwrap().into(),
            schedule: vec![(2, "income-tax".into(), "0.2".into())],
            ..Config::default()
        };
        let mut original = World::new(cfg).unwrap();
        for _ in 0..35 {
            original.tick_day();
        }
        for x in 0..3 {
            original.city.set(x, 1, crate::city::Zone::Road);
        }
        let tile = original
            .city
            .tiles
            .iter()
            .position(|tile| tile.zone == crate::city::Zone::Empty)
            .unwrap();
        let (x, y) = original.city.xy(tile as u16);
        let funding = 1_000_000 - original.ledger.balance(original.gov.account);
        if funding > 0 {
            original.ledger.mint(original.gov.account, funding);
        }
        original
            .start_construction(x as usize, y as usize, crate::city::Zone::Residential)
            .unwrap();
        assert_eq!(original.construction.len(), 1);
        for &firm in &original.firms.active_list {
            let firm = firm as usize;
            if original.firms.good[firm] as usize == crate::goods::SHELTER {
                original.firms.inventory[firm] = 1000;
                original.firms.effective_labor[firm] = 100.0;
            }
        }
        original.construction_day();
        assert_eq!(original.construction.len(), 1);
        let project = &original.construction[0];
        assert!(project.materials_delivered > 0 && project.spent > 0);
        assert!(project.work_done > 0.0 && project.work_done < project.work_required);
        let escrow = project.escrow;
        let reserved = original.ledger.balance(escrow);
        let path = dir.join("world.save");
        original.save(&path).unwrap();
        let mut resumed = World::load(&path, dir.join("resumed")).unwrap();
        assert_eq!(original.stats.hash(), resumed.stats.hash());
        assert_eq!(resumed.ledger.balance(escrow), reserved);
        for _ in 0..3 {
            original.construction_day();
            resumed.construction_day();
        }
        assert!(original.construction.is_empty() && resumed.construction.is_empty());
        assert_eq!(original.ledger.balance(escrow), 0);
        assert_eq!(resumed.ledger.balance(escrow), 0);
        assert_eq!(
            original.city.tiles[tile].zone,
            crate::city::Zone::Residential
        );
        assert_eq!(
            resumed.city.tiles[tile].zone,
            crate::city::Zone::Residential
        );
        for _ in 0..370 {
            original.tick_day();
            resumed.tick_day();
        }
        assert_eq!(original.stats.hash(), resumed.stats.hash());
        // Compare every serialized field, including RNGs and policy schedules.
        resumed.cfg.out_dir = original.cfg.out_dir.clone();
        assert_eq!(
            codec().serialize(&original).unwrap(),
            codec().serialize(&resumed).unwrap()
        );
        assert!(World::load(&path, dir.join("original")).is_err());
        let mut corrupt = fs::read(&path).unwrap();
        *corrupt.last_mut().unwrap() ^= 1;
        fs::write(dir.join("corrupt.save"), corrupt).unwrap();
        let error = World::load(dir.join("corrupt.save"), dir.join("corrupt-output"))
            .err()
            .unwrap();
        assert!(error.to_string().contains("checksum"));
        assert!(!dir.join("corrupt-output").exists());
        drop(original);
        drop(resumed);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn traffic_snapshot_preserves_assignment_across_topology_and_month_boundary() {
        use crate::{city::Zone, traffic::TripKind};
        let dir = directory();
        let mut original = World::new(Config {
            n_hh: 40,
            n_firms: 10,
            quiet: true,
            out_dir: dir.join("original").to_str().unwrap().into(),
            ..Config::default()
        })
        .unwrap();
        for _ in 0..27 {
            original.tick_day();
        }
        // Warm the derived caches and establish a frozen monthly route, then
        // invalidate between ticks as a player demolition would.
        let a = original.city.idx(3, 0);
        let b = original.city.idx(3, 6);
        original.city.ensure_route(a, b);
        original.city.traffic.add_trip(a, b, TripKind::Freight, 7);
        original.city.set(3, 2, Zone::Empty);
        let path = dir.join("traffic.save");
        original.save(&path).unwrap();
        // The dense tile-pair accelerator alone would exceed nine megabytes.
        assert!(fs::metadata(&path).unwrap().len() < 4_000_000);
        let mut resumed = World::load(&path, dir.join("resumed")).unwrap();
        for day in 0..35 {
            if day == 3 {
                original.city.set(3, 2, Zone::Road);
                resumed.city.set(3, 2, Zone::Road);
            }
            original.tick_day();
            resumed.tick_day();
            assert_eq!(original.city.traffic.daily, resumed.city.traffic.daily);
            assert_eq!(original.city.traffic.flows, resumed.city.traffic.flows);
        }
        resumed.cfg.out_dir = original.cfg.out_dir.clone();
        assert_eq!(
            codec().serialize(&original).unwrap(),
            codec().serialize(&resumed).unwrap()
        );
        // A second snapshot with live, frozen paths exercises reconstruction of
        // pair lookups without throwing those paths away.
        original.save(&path).unwrap();
        let mut again = World::load(&path, dir.join("again")).unwrap();
        for _ in 0..30 {
            original.tick_day();
            again.tick_day();
        }
        again.cfg.out_dir = original.cfg.out_dir.clone();
        assert_eq!(
            codec().serialize(&original).unwrap(),
            codec().serialize(&again).unwrap()
        );
        drop(original);
        drop(resumed);
        drop(again);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn opposition_snapshot_preserves_platform_authority_and_live_world_on_failure() {
        use crate::commands::Command;
        let dir = directory();
        let mut live = World::new(Config {
            n_hh: 40,
            n_firms: 10,
            firm_cap: 20,
            quiet: true,
            out_dir: dir.join("live").to_str().unwrap().into(),
            ..Config::default()
        })
        .unwrap();
        live.start_player_session();
        live.gov.player_in_power = false;
        let incumbent = live.gov.policy.clone();
        live.apply_command(Command::SetPolicy {
            key: "income-tax".into(),
            value: "0.37".into(),
        })
        .unwrap();
        let platform = live.gov.player_policy.clone();
        assert_ne!(platform.as_ref(), Some(&incumbent));
        let path = dir.join("opposition.save");
        live.save(&path).unwrap();
        let mut loaded = World::load(&path, dir.join("loaded")).unwrap();
        let events_before = loaded.events.count;
        loaded.start_player_session();
        assert!(!loaded.gov.player_in_power);
        assert_eq!(loaded.gov.policy, incumbent);
        assert_eq!(loaded.gov.player_policy, platform);
        assert_eq!(loaded.events.count, events_before);
        let before = codec().serialize(&live).unwrap();
        // Frontends replace their world only after a successful load.
        if let Ok(replacement) = World::load(&path, dir.join("live")) {
            live = replacement;
        }
        assert_eq!(codec().serialize(&live).unwrap(), before);
        drop(live);
        drop(loaded);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rejects_malformed_and_unsupported_versions_without_creating_output() {
        let dir = directory();
        let path = dir.join("bad.save");
        for bytes in [
            vec![],
            b"not a snapshot".to_vec(),
            [MAGIC.as_slice(), &1u32.to_le_bytes()].concat(),
            [MAGIC.as_slice(), &999u32.to_le_bytes()].concat(),
            [MAGIC.as_slice(), &VERSION.to_le_bytes(), &[255, 255]].concat(),
        ] {
            fs::write(&path, bytes).unwrap();
            let error = World::load(&path, dir.join("output")).err().unwrap();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(!dir.join("output").exists());
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
