//! Interactive mode: the world runs on its own thread, paced by a speed
//! setting; a dependency-free HTTP server hands the browser a JSON snapshot
//! and takes the player's decisions back. The player is the government; the
//! voters judge them every four years.

use crate::politics::{N_PARTIES, PARTIES};
use crate::sim::{Config, World, DAYS_PER_MONTH, DAYS_PER_YEAR, MONTHS_PER_YEAR};
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

pub fn serve(cfg: Config, port: u16) -> std::io::Result<()> {
    let shared = Arc::new(Shared {
        control: Mutex::new(Control { speed: 7.0, paused: true, step_days: 0, pending: Vec::new() }),
        snapshot: Mutex::new(String::from("{\"loading\":true}")),
    });
    let s2 = shared.clone();
    std::thread::spawn(move || sim_loop(cfg, s2));
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("econsim is serving at http://127.0.0.1:{}  (Ctrl-C to stop)", port);
    for stream in listener.incoming() {
        if let Ok(stream) = stream {
            let sh = shared.clone();
            std::thread::spawn(move || handle(stream, sh));
        }
    }
    Ok(())
}

fn sim_loop(cfg: Config, shared: Arc<Shared>) {
    let mut world = World::new(cfg).expect("world");
    world.gov.player_policy = Some(world.gov.policy.clone());
    world.gov.player_in_power = true;
    world.events.log(0, "player", "interactive mode: you are the government; elections every 4 years judge your bundle");
    let mut approval: Vec<(u32, f64, f64)> = Vec::new(); // (month, government share, mean preference)
    let mut last_month = u32::MAX;
    let mut carry = 0.0f64;
    let mut last = Instant::now();
    let mut dirty = true;
    loop {
        let (speed, paused, mut n, pending) = {
            let mut c = shared.control.lock().unwrap();
            let p = std::mem::take(&mut c.pending);
            let n = c.step_days;
            c.step_days = 0;
            (c.speed, c.paused, n, p)
        };
        if !pending.is_empty() {
            let mut player = world.gov.player_policy.clone().unwrap_or_default();
            let mut changed = Vec::new();
            for (k, v) in &pending {
                if k == "build" {
                    // "x,y,zone" from the city palette
                    let parts: Vec<&str> = v.split(',').collect();
                    if parts.len() == 3 {
                        if let (Ok(x), Ok(y), Ok(z)) = (parts[0].parse::<usize>(), parts[1].parse::<usize>(), parts[2].parse::<u8>()) {
                            world.gov.pending_builds.push((x, y, z));
                        }
                    }
                    continue;
                }
                if k == "print" {
                    // one-off money creation (or destruction) is always the treasury's to do
                    let amt: i64 = v.parse().unwrap_or(0);
                    world.gov.pending_print += amt;
                    world.events.log(world.day, "player", &format!("government orders {} of {} cents at month end", if amt >= 0 { "printing" } else { "burning" }, amt.abs()));
                    continue;
                }
                if player.set(k, v).is_ok() {
                    changed.push(format!("{}={}", k, v));
                }
            }
            if changed.is_empty() {
                // builds and prints are applied at month end; step to it if paused so the player sees it
                dirty = true;
                if paused && !world.gov.pending_builds.is_empty() {
                    let mut c = shared.control.lock().unwrap();
                    c.step_days += DAYS_PER_MONTH - (world.day % DAYS_PER_MONTH);
                }
                continue;
            }
            world.gov.player_policy = Some(player.clone());
            if world.gov.player_in_power {
                world.gov.policy = player;
                world.events.log(world.day, "player", &format!("government sets {}", changed.join(", ")));
            } else {
                world.events.log(world.day, "player", &format!("opposition platform now {} (in force after the next election win)", changed.join(", ")));
            }
            dirty = true;
        }
        let now = Instant::now();
        let dt = now.duration_since(last).as_secs_f64();
        last = now;
        if !paused {
            if speed.is_infinite() {
                n += DAYS_PER_MONTH;
            } else {
                carry += speed * dt;
                let whole = carry.floor();
                carry -= whole;
                n += whole as u32;
            }
        }
        n = n.min(DAYS_PER_YEAR);
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
            let json = snapshot(&world, speed, paused, &approval);
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
    if v.is_finite() { format!("{:.4}", v) } else { "null".to_string() }
}

fn snapshot(w: &World, speed: f64, paused: bool, approval: &[(u32, f64, f64)]) -> String {
    let pol = &w.gov.policy;
    let player = w.gov.player_policy.clone().unwrap_or_default();
    let (shares, mean_pref) = w.poll();
    let d = w.stats.last_daily.clone();
    let months = &w.stats.months;
    let last = months.last();
    let mut o = String::with_capacity(200_000);
    o.push('{');
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
    o.push_str(&format!("\"policy\":{},\"player_policy\":{},", pol_json(pol), pol_json(&player)));
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
        o.push_str(&format!(",\"loan_rate\":{},\"printed\":{},\"printed_total\":{},\"deposit_interest\":{}", num(m.loan_rate), m.printed, w.bank.printed_total, m.deposit_interest));
        o.push_str(&format!(
            ",\"housing_capacity\":{},\"homes_housed\":{},\"unhoused\":{},\"business_slots\":{},\"business_used\":{},\"school_capacity\":{},\"avg_land_value\":{},\"rent_revenue\":{},\"avg_commute\":{}",
            m.housing_capacity, m.homes_housed, m.unhoused, m.business_slots, m.business_used, m.school_capacity, m.avg_land_value, m.rent_revenue, num(m.avg_commute)
        ));
    }
    o.push_str("},");
    // city grid: [zone, occupants, land value] per tile
    o.push_str(&format!("\"city\":{{\"w\":{},\"h\":{},\"tiles\":[", w.city.w, w.city.h));
    for (i, t) in w.city.tiles.iter().enumerate() {
        if i > 0 {
            o.push(',');
        }
        o.push_str(&format!("[{},{},{}]", t.zone as u8, t.occupants, t.land_value));
    }
    o.push_str(&format!("],\"costs\":[0,{},{},{},{}],\"capacity\":[0,{},{},{},0]}},",
        crate::city::Zone::Residential.cost(), crate::city::Zone::Business.cost(), crate::city::Zone::School.cost(), crate::city::Zone::Park.cost(),
        crate::city::Zone::Residential.capacity(), crate::city::Zone::Business.capacity(), crate::city::Zone::School.capacity()));
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
    series!("unemployment", |m: &crate::stats::MonthRow| num(m.unemployment));
    series!("cpi", |m: &crate::stats::MonthRow| num(m.price_index));
    series!("inflation", |m: &crate::stats::MonthRow| num(m.inflation));
    series!("gini", |m: &crate::stats::MonthRow| num(m.gini));
    series!("real_wage", |m: &crate::stats::MonthRow| num(m.real_wage));
    series!("debt", |m: &crate::stats::MonthRow| m.gov_debt.to_string());
    series!("treasury", |m: &crate::stats::MonthRow| m.gov_cash.to_string());
    series!("money", |m: &crate::stats::MonthRow| m.money_supply.to_string());
    series!("population", |m: &crate::stats::MonthRow| m.population.to_string());
    series!("taxes", |m: &crate::stats::MonthRow| m.taxes.to_string());
    series!("transfers", |m: &crate::stats::MonthRow| (m.benefits + m.universal_dividend + m.pensions + m.child_benefit + m.education_spend).to_string());
    series!("firms", |m: &crate::stats::MonthRow| m.firms.to_string());
    series!("market_cap", |m: &crate::stats::MonthRow| m.market_cap.to_string());
    series!("mean_pref", |m: &crate::stats::MonthRow| num(m.mean_pref));
    series!("public_quality", |m: &crate::stats::MonthRow| num(m.public_quality));
    series!("policy_rate", |m: &crate::stats::MonthRow| num(m.policy_rate));
    series!("loan_rate", |m: &crate::stats::MonthRow| num(m.loan_rate));
    series!("bond_rate", |m: &crate::stats::MonthRow| num(m.bond_rate));
    series!("printed", |m: &crate::stats::MonthRow| m.printed.to_string());
    series!("unhoused", |m: &crate::stats::MonthRow| m.unhoused.to_string());
    series!("homes", |m: &crate::stats::MonthRow| m.homes.to_string());
    series!("housing_capacity", |m: &crate::stats::MonthRow| m.housing_capacity.to_string());
    series!("avg_land_value", |m: &crate::stats::MonthRow| m.avg_land_value.to_string());
    series!("rent_revenue", |m: &crate::stats::MonthRow| m.rent_revenue.to_string());
    series!("immigrants", |m: &crate::stats::MonthRow| m.immigrants.to_string());
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
        o.push_str(&format!("{{\"day\":{},\"kind\":\"{}\",\"msg\":\"{}\"}}", day, esc(kind), esc(msg)));
    }
    o.push_str("],");
    o.push_str(&format!("\"parties\":[{}]", PARTIES.iter().map(|(n, p)| format!("[\"{}\",{}]", n, p)).collect::<Vec<_>>().join(",")));
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
        if let Some(v) = l.strip_prefix("Content-Length:").or_else(|| l.strip_prefix("content-length:")) {
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
        ("GET", "/") | ("GET", "/index.html") => ("200 OK", "text/html; charset=utf-8", UI.to_string()),
        ("GET", "/state") => ("200 OK", "application/json", shared.snapshot.lock().unwrap().clone()),
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
                    c.speed = if value == "max" { f64::INFINITY } else { value.parse().unwrap_or(7.0) };
                    c.paused = false;
                }
                "step" => c.step_days += value.parse::<u32>().unwrap_or(1),
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
