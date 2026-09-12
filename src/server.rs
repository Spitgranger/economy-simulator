//! Interactive mode: the world runs on its own thread, paced by a speed
//! setting; a dependency-free HTTP server hands the browser a JSON snapshot
//! and takes the player's decisions back. The player is the government; the
//! voters judge them every four years.

use crate::commands::{Command, SimulationClock};
use crate::politics::{N_PARTIES, PARTIES};
use crate::sim::{World, DAYS_PER_MONTH, DAYS_PER_YEAR, MONTHS_PER_YEAR};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const UI: &str = include_str!("ui.html");

struct Control {
    speed: f64, // simulated days per real second; INFINITY = as fast as possible
    paused: bool,
    step_days: u32,
    pending: Vec<(String, String)>,
}

struct Shared {
    control: Mutex<Control>,
    snapshot: Mutex<String>,
}

pub fn serve(world: World, port: u16, save: Option<String>) -> std::io::Result<()> {
    let shared = Arc::new(Shared {
        control: Mutex::new(Control {
            speed: 7.0,
            paused: true,
            step_days: 0,
            pending: Vec::new(),
        }),
        snapshot: Mutex::new(String::from("{\"loading\":true}")),
    });
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let s2 = shared.clone();
    let save = save.unwrap_or_else(|| format!("{}/city.econsave", world.cfg.out_dir));
    std::thread::spawn(move || sim_loop(world, s2, save));
    println!(
        "econsim is serving at http://127.0.0.1:{}  (Ctrl-C to stop)",
        port
    );
    for stream in listener.incoming() {
        if let Ok(stream) = stream {
            let sh = shared.clone();
            std::thread::spawn(move || handle(stream, sh));
        }
    }
    Ok(())
}

fn parse_command(k: &str, v: &str) -> Result<Command, String> {
    match k {
        "build" => {
            let parts: Vec<&str> = v.split(',').collect();
            if parts.len() != 3 {
                return Err("build expects x,y,zone".into());
            }
            let x = parts[0].parse().map_err(|_| "invalid x")?;
            let y = parts[1].parse().map_err(|_| "invalid y")?;
            let z: u8 = parts[2].parse().map_err(|_| "invalid zone")?;
            if z > 5 {
                return Err("invalid zone".into());
            }
            Ok(Command::Build {
                x,
                y,
                zone: crate::city::Zone::from_u8(z),
            })
        }
        "cancel" => {
            let (x, y) = v.split_once(',').ok_or("cancel expects x,y")?;
            Ok(Command::CancelBuild {
                x: x.parse().map_err(|_| "invalid x")?,
                y: y.parse().map_err(|_| "invalid y")?,
            })
        }
        "cancel-all" => Ok(Command::CancelAllQueuedBuilds),
        "print" => Ok(Command::PrintMoney {
            amount: v.parse().map_err(|_| "invalid money amount")?,
        }),
        _ => Ok(Command::SetPolicy {
            key: k.into(),
            value: v.into(),
        }),
    }
}

fn sim_loop(mut world: World, shared: Arc<Shared>, save: String) {
    world.start_player_session();
    let mut approval: Vec<(u32, f64, f64)> = Vec::new();
    let mut last_month = u32::MAX;
    let mut clock = SimulationClock::new(DAYS_PER_MONTH);
    let mut last = Instant::now();
    let mut dirty = true;
    let mut previous_control = (7.0, true);
    let mut message =
        "Construction is funded at month end, then needs materials and labor.".to_string();
    loop {
        let (speed, paused, step_days, pending) = {
            let mut c = shared.control.lock().unwrap();
            let p = std::mem::take(&mut c.pending);
            let n = std::mem::take(&mut c.step_days);
            (c.speed, c.paused, n, p)
        };
        dirty |= previous_control != (speed, paused);
        previous_control = (speed, paused);
        if !pending.is_empty() {
            for (k, v) in pending {
                if k == "save" {
                    message = match world.save(&save) {
                        Ok(()) => format!("Saved to {save}"),
                        Err(e) => format!("Save failed: {e}"),
                    };
                } else {
                    message = match parse_command(&k, &v)
                        .and_then(|c| world.apply_command(c).map_err(|e| e.to_string()))
                    {
                        Ok(outcome) => outcome.message,
                        Err(e) => format!("Rejected: {e}"),
                    };
                }
            }
            dirty = true;
        }
        let now = Instant::now();
        let n = clock.advance(now.duration_since(last), speed, paused, step_days);
        last = now;
        for _ in 0..n {
            world.tick_day();
            let month = world.day / DAYS_PER_MONTH;
            if world.day % DAYS_PER_MONTH == 0 && month != last_month {
                last_month = month;
                let (shares, mean) = world.poll();
                approval.push((month, shares[N_PARTIES], mean));
            }
        }
        if n > 0 || dirty {
            let json = snapshot(&world, speed, paused, &approval, &message);
            *shared.snapshot.lock().unwrap() = json;
            dirty = false;
        }
        if n == 0 {
            std::thread::sleep(Duration::from_millis(20));
        }
        if world.day % DAYS_PER_YEAR == 0 && n > 0 {
            let _ = world.finish();
        }
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => {}
            c => o.push(c),
        }
    }
    o
}

fn num(v: f64) -> String {
    if v.is_finite() {
        format!("{:.4}", v)
    } else {
        "null".to_string()
    }
}

fn snapshot(
    w: &World,
    speed: f64,
    paused: bool,
    approval: &[(u32, f64, f64)],
    message: &str,
) -> String {
    let pol = &w.gov.policy;
    let player = w.gov.player_policy.clone().unwrap_or_default();
    let (shares, mean_pref) = w.poll();
    let d = w.stats.last_daily.clone();
    let months = &w.stats.months;
    let last = months.last();
    let mut o = String::with_capacity(200_000);
    o.push('{');
    o.push_str(&format!("\"command_message\":\"{}\",", esc(message)));
    o.push_str(&format!(
        "\"day\":{},\"year\":{},\"month\":{},\"speed\":{},\"paused\":{},\"player_in_power\":{},\"locked_until\":{},\"incumbent\":{},\"elections\":{},",
        w.day,
        w.day / DAYS_PER_YEAR,
        (w.day / DAYS_PER_MONTH) % MONTHS_PER_YEAR,
        if speed.is_infinite() { "\"max\"".to_string() } else { num(speed) },
        paused,
        w.gov.player_in_power,
        w.gov.locked_until,
        w.gov.incumbent.map(|k| format!("\"{}\"", PARTIES[k].0)).unwrap_or("null".into()),
        w.gov.elections_held
    ));
    let pol_json = |p: &crate::government::Policy| {
        format!(
            "{{\"income-tax\":{},\"sales-tax\":{},\"luxury-tax\":{},\"dividend-tax\":{},\"inheritance-tax\":{},\"benefit\":{},\"basic-income\":{},\"pension\":{},\"child-benefit\":{},\"min-wage\":{},\"class-size\":{},\"teacher-pay\":{},\"surplus-dividend\":{},\"policy-rate\":{},\"print-rate\":{},\"ground-rent\":{},\"position\":{}}}",
            num(p.income_tax), num(p.sales_tax[0]), num(p.sales_tax[crate::goods::LUXURY]), num(p.dividend_tax), num(p.inheritance_tax),
            num(p.benefit_rate), num(p.basic_income), num(p.pension), num(p.child_benefit), p.min_wage, num(p.class_size), num(p.teacher_pay),
            num(p.surplus_dividend), num(p.policy_rate), num(p.print_rate), num(p.ground_rent), num(p.position())
        )
    };
    o.push_str(&format!(
        "\"policy\":{},\"player_policy\":{},",
        pol_json(pol),
        pol_json(&player)
    ));
    o.push_str(&format!(
        "\"poll\":{{\"Market\":{},\"Centre\":{},\"Labour\":{},\"Government\":{}}},\"mean_pref\":{},",
        num(shares[0]), num(shares[1]), num(shares[2]), num(shares[3]), num(mean_pref)
    ));
    // now
    o.push_str("\"now\":{");
    if let Some(d) = &d {
        o.push_str(&format!(
            "\"unemployment\":{},\"cpi\":{},\"employed\":{},\"population\":{},\"money\":{},\"loans\":{},\"debt\":{},\"treasury\":{},\"firms\":{},\"wage\":{},\"hh_cash\":{},\"firm_cash\":{}",
            num(d.unemployment), num(d.price_index), d.employed, d.population, d.money_supply, d.loans, d.gov_debt, d.gov_cash, d.firms, num(d.wage), d.hh_cash, d.firm_cash
        ));
    }
    if let Some(m) = last {
        o.push_str(&format!(
            ",\"inflation\":{},\"gini\":{},\"top10\":{},\"real_wage\":{},\"homes\":{},\"couples\":{},\"minors\":{},\"retirees\":{},\"pupils_public\":{},\"pupils_private\":{},\"public_quality\":{},\"policy_rate\":{},\"bond_rate\":{},\"market_cap\":{},\"life_expectancy\":{},\"mobility\":{},\"taxes\":{},\"transfers\":{},\"mean_skill\":{}",
            num(m.inflation), num(m.gini), num(m.top10_share), num(m.real_wage), m.homes, m.couples, m.minors, m.retirees, m.pupils_public, m.pupils_private,
            num(m.public_quality), num(m.policy_rate), num(m.bond_rate), m.market_cap, num(m.life_expectancy), num(m.mobility_corr), m.taxes,
            m.benefits + m.universal_dividend + m.pensions + m.child_benefit + m.education_spend, num(m.mean_skill)
        ));
        o.push_str(&format!(
            ",\"loan_rate\":{},\"printed\":{},\"printed_total\":{},\"deposit_interest\":{}",
            num(m.loan_rate),
            m.printed,
            w.bank.printed_total,
            m.deposit_interest
        ));
        o.push_str(&format!(
            ",\"housing_capacity\":{},\"homes_housed\":{},\"unhoused\":{},\"business_slots\":{},\"business_used\":{},\"school_capacity\":{},\"avg_land_value\":{},\"rent_revenue\":{},\"avg_commute\":{}",
            m.housing_capacity, m.homes_housed, m.unhoused, m.business_slots, m.business_used, m.school_capacity, m.avg_land_value, m.rent_revenue, num(m.avg_commute)
        ));
    }
    o.push_str("},");
    // city grid: [zone, occupants, land value] per tile
    o.push_str(&format!(
        "\"city\":{{\"w\":{},\"h\":{},\"tiles\":[",
        w.city.w, w.city.h
    ));
    for (i, t) in w.city.tiles.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        o.push_str(&format!(
            "[{},{},{}]",
            t.zone as u8, t.occupants, t.land_value
        ));
    }
    o.push_str(&format!(
        "],\"costs\":[0,{},{},{},{},60000],\"capacity\":[0,{},{},{},0,0]}},",
        crate::city::Zone::Residential.cost(),
        crate::city::Zone::Business.cost(),
        crate::city::Zone::School.cost(),
        crate::city::Zone::Park.cost(),
        crate::city::Zone::Residential.capacity(),
        crate::city::Zone::Business.capacity(),
        crate::city::Zone::School.capacity()
    ));
    o.push_str(&format!("\"traffic\":{{\"capacity\":400,\"flows\":{:?},\"access\":{:?},\"requested\":{:?},\"routed\":{:?},\"off_network\":{:?}}},", w.city.traffic.flows, (0..w.city.tiles.len()).map(|t|w.city.road_access(t as u16)).collect::<Vec<_>>(), w.city.traffic.daily.requested, w.city.traffic.daily.routed, w.city.traffic.daily.off_network));
    o.push_str("\"projects\":[");
    for (i, p) in w.construction.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        let (x, y) = w.city.xy(p.tile);
        o.push_str(&format!("{{\"x\":{x},\"y\":{y},\"zone\":{},\"progress\":{},\"materials\":{},\"required\":{},\"spent\":{},\"budget\":{},\"status\":\"{}\"}}",p.zone as u8,num(p.progress()),p.materials_delivered,p.materials_required,p.spent,p.budget,p.stall.name()));
    }
    o.push_str("],\"queued_builds\":[");
    for (i, &(x, y, z)) in w.gov.pending_builds.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        o.push_str(&format!("[{x},{y},{z}]"));
    }
    o.push_str("],");
    // series
    macro_rules! series {
        ($name:expr, $f:expr) => {{
            o.push_str(&format!("\"{}\":[", $name));
            for (i, m) in months.iter().enumerate() {
                if i > 0 {
                    o.push(',');
                }
                o.push_str(&$f(m));
            }
            o.push_str("],");
        }};
    }
    o.push_str("\"series\":{");
    series!("month", |m: &crate::stats::MonthRow| m.month.to_string());
    series!("unemployment", |m: &crate::stats::MonthRow| num(
        m.unemployment
    ));
    series!("cpi", |m: &crate::stats::MonthRow| num(m.price_index));
    series!("inflation", |m: &crate::stats::MonthRow| num(m.inflation));
    series!("gini", |m: &crate::stats::MonthRow| num(m.gini));
    series!("real_wage", |m: &crate::stats::MonthRow| num(m.real_wage));
    series!("debt", |m: &crate::stats::MonthRow| m.gov_debt.to_string());
    series!("treasury", |m: &crate::stats::MonthRow| m
        .gov_cash
        .to_string());
    series!("money", |m: &crate::stats::MonthRow| m
        .money_supply
        .to_string());
    series!("population", |m: &crate::stats::MonthRow| m
        .population
        .to_string());
    series!("taxes", |m: &crate::stats::MonthRow| m.taxes.to_string());
    series!("transfers", |m: &crate::stats::MonthRow| (m.benefits
        + m.universal_dividend
        + m.pensions
        + m.child_benefit
        + m.education_spend)
        .to_string());
    series!("firms", |m: &crate::stats::MonthRow| m.firms.to_string());
    series!("market_cap", |m: &crate::stats::MonthRow| m
        .market_cap
        .to_string());
    series!("mean_pref", |m: &crate::stats::MonthRow| num(m.mean_pref));
    series!("public_quality", |m: &crate::stats::MonthRow| num(
        m.public_quality
    ));
    series!("policy_rate", |m: &crate::stats::MonthRow| num(
        m.policy_rate
    ));
    series!("loan_rate", |m: &crate::stats::MonthRow| num(m.loan_rate));
    series!("bond_rate", |m: &crate::stats::MonthRow| num(m.bond_rate));
    series!("printed", |m: &crate::stats::MonthRow| m
        .printed
        .to_string());
    series!("unhoused", |m: &crate::stats::MonthRow| m
        .unhoused
        .to_string());
    series!("homes", |m: &crate::stats::MonthRow| m.homes.to_string());
    series!("housing_capacity", |m: &crate::stats::MonthRow| m
        .housing_capacity
        .to_string());
    series!("avg_land_value", |m: &crate::stats::MonthRow| m
        .avg_land_value
        .to_string());
    series!("rent_revenue", |m: &crate::stats::MonthRow| m
        .rent_revenue
        .to_string());
    series!("immigrants", |m: &crate::stats::MonthRow| m
        .immigrants
        .to_string());
    o.push_str("\"approval\":[");
    for (i, (_, a, _)) in approval.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        o.push_str(&num(*a));
    }
    o.push_str("]},");
    // events
    o.push_str("\"events\":[");
    for (i, (day, kind, msg)) in w.events.recent.iter().rev().take(40).enumerate() {
        if i > 0 {
            o.push(',');
        }
        o.push_str(&format!(
            "{{\"day\":{},\"kind\":\"{}\",\"msg\":\"{}\"}}",
            day,
            esc(kind),
            esc(msg)
        ));
    }
    o.push_str("],");
    o.push_str(&format!(
        "\"parties\":[{}]",
        PARTIES
            .iter()
            .map(|(n, p)| format!("[\"{}\",{}]", n, p))
            .collect::<Vec<_>>()
            .join(",")
    ));
    o.push('}');
    o
}

fn handle(mut stream: TcpStream, shared: Arc<Shared>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end;
    loop {
        let n = match stream.read(&mut tmp) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
        if buf.len() > 65536 {
            return;
        }
    }
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request = lines.next().unwrap_or("");
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut content_length = 0usize;
    for l in lines {
        if let Some(v) = l
            .strip_prefix("Content-Length:")
            .or_else(|| l.strip_prefix("content-length:"))
        {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = buf[header_end..].to_vec();
    while body.len() < content_length {
        let n = match stream.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        body.extend_from_slice(&tmp[..n]);
    }
    let body = String::from_utf8_lossy(&body).to_string();

    let (status, ctype, out) = match (method.as_str(), path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => {
            ("200 OK", "text/html; charset=utf-8", UI.to_string())
        }
        ("GET", "/state") => (
            "200 OK",
            "application/json",
            shared.snapshot.lock().unwrap().clone(),
        ),
        ("POST", "/policy") => {
            let mut c = shared.control.lock().unwrap();
            for kv in body.split('&') {
                if let Some((k, v)) = kv.split_once('=') {
                    c.pending.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            ("200 OK", "application/json", "{\"ok\":true}".to_string())
        }
        ("POST", "/control") => {
            let mut action = String::new();
            let mut value = String::new();
            for kv in body.split('&') {
                if let Some((k, v)) = kv.split_once('=') {
                    match k.trim() {
                        "action" => action = v.trim().to_string(),
                        "value" => value = v.trim().to_string(),
                        _ => {}
                    }
                }
            }
            let mut c = shared.control.lock().unwrap();
            match action.as_str() {
                "play" => c.paused = false,
                "pause" => c.paused = true,
                "speed" => {
                    c.speed = if value == "max" {
                        f64::INFINITY
                    } else {
                        value
                            .parse::<f64>()
                            .ok()
                            .filter(|v| v.is_finite() && *v > 0.0 && *v <= 336.0)
                            .unwrap_or(7.0)
                    };
                    c.paused = false;
                }
                "step" => {
                    c.step_days = c
                        .step_days
                        .saturating_add(value.parse::<u32>().unwrap_or(1))
                        .min(DAYS_PER_YEAR)
                }
                _ => {}
            }
            ("200 OK", "application/json", "{\"ok\":true}".to_string())
        }
        _ => ("404 Not Found", "text/plain", "not found".to_string()),
    };
    let resp = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status, ctype, out.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.flush();
}
