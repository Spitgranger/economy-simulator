mod bank;
mod city;
mod demographics;
mod education;
mod finance;
mod firms;
mod goods;
mod government;
mod homes;
mod ledger;
mod people;
mod politics;
mod rng;
mod server;
mod sim;
mod stats;

use sim::{Config, World, DAYS_PER_YEAR, MONTHS_PER_YEAR};

fn usage() -> ! {
    eprintln!(
        "econsim - agent-based economy simulator

USAGE: econsim [options]            run headless, write CSVs and logs
       econsim play [options]       desktop mode: start the game and open it in a browser window
       econsim serve [options]      same, without opening a browser (http://127.0.0.1:8080)
  --no-browser        with play: do not launch a browser
  --port N            port for serve (default 8080)
  --seed N            PRNG seed (default 42)
  --years N           simulated years (default 20)
  --days N            simulated days (overrides --years)
  --households N      number of households (default 1000)
  --firms N           initial firms (default 100)
  --workers N         initial workers per firm (default 8)
  --money CENTS       initial cash per household (default 20000)
  --out DIR           output directory (default out)
  --watch-firm N      trace firm N in watch.log (default 0, -1 = none)
  --watch-home N      trace home N in watch.log (default 0, -1 = none)
  --watch-person N    trace person N in watch.log (default 0, -1 = none)
  --replay-check      run twice with the same seed and verify identical output
  --quiet             no yearly progress lines

Government levers (all default to 0 except surplus-dividend = 1):
  --income-tax R      withheld from wages at payroll
  --sales-tax R       added to every purchase price
  --luxury-tax R      sales tax on the luxury good (overrides --sales-tax for it)
  --dividend-tax R    withheld from dividends
  --benefit R         weekly unemployment benefit as a fraction of the average wage
  --basic-income R    weekly payment to every adult as a fraction of the average wage
  --pension R         weekly payment to every retiree (65+) as a fraction of the average wage
  --child-benefit R   weekly payment per minor child to the parent; also public education quality
  --inheritance-tax R share of a cash estate taken at death
  --class-size N      public school target pupils per teacher (0 = no public school)
  --teacher-pay R     public teacher pay as a multiple of the average wage (default 1)
  --min-wage CENTS    floor on gross weekly pay
  --policy-rate R|auto central bank rate set by hand (annual), or auto = Taylor rule (default)
  --print-rate R      new money minted into the treasury each month, share of the money supply
  --ground-rent R     monthly ground rent as a share of land value (homes x1, firms x2; default 0.01)
  --surplus-dividend R share of treasury above one month of benefits returned
                      equally to all households each month
  --policy-at Y:key=v change a lever at the start of year Y (repeatable), e.g.
                      --policy-at 5:benefit=0.4 --policy-at 5:income-tax=0.15

Politics and finance:
  --no-elections      keep the levers as set above (default: elections every 4 years
                      from year 1 between Market / Centre / Labour platforms)
  --election-every N  years between elections (default 4)
  --first-election N  year of the first election (default 1)
  --no-credit         bank neither lends to firms nor buys bonds (fixed money stock)
  --no-stock-market   shares are quoted but never traded
  --no-demographics   nobody ages, is born or dies (all adults aged 30)
  --bank-equity CENTS bank equity target; lending capacity is 10x (default 1000000)
"
    );
    std::process::exit(2)
}

/// Open the UI as an application window in whatever browser the desktop has.
fn open_browser(url: &str) {
    let attempts: Vec<(&str, Vec<String>)> = if cfg!(target_os = "windows") {
        vec![("cmd", vec!["/C".into(), "start".into(), "".into(), url.to_string()])]
    } else if cfg!(target_os = "macos") {
        vec![("open", vec![url.to_string()])]
    } else {
        vec![
            ("chromium", vec![format!("--app={}", url)]),
            ("chromium-browser", vec![format!("--app={}", url)]),
            ("google-chrome", vec![format!("--app={}", url)]),
            ("firefox", vec!["--new-window".into(), url.to_string()]),
            ("xdg-open", vec![url.to_string()]),
        ]
    };
    for (bin, args) in attempts {
        if std::process::Command::new(bin).args(&args).spawn().is_ok() {
            return;
        }
    }
    eprintln!("could not find a browser to open; visit {}", url);
}

fn parse_args(args: &[String]) -> (Config, bool, u16) {
    let mut cfg = Config::default();
    let mut replay = false;
    let mut years: Option<u32> = None;
    let mut port: u16 = 8080;
    let args: Vec<String> = args.to_vec();
    let mut i = 0;
    let val = |i: &mut usize| -> String {
        *i += 1;
        args.get(*i).cloned().unwrap_or_else(|| usage())
    };
    let num = |s: String| -> f64 { s.parse().unwrap_or_else(|_| usage()) };
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--seed" => cfg.seed = num(val(&mut i)) as u64,
            "--years" => years = Some(num(val(&mut i)) as u32),
            "--days" => cfg.days = num(val(&mut i)) as u32,
            "--households" => cfg.n_hh = num(val(&mut i)) as usize,
            "--firms" => cfg.n_firms = num(val(&mut i)) as usize,
            "--workers" => cfg.workers_per_firm = num(val(&mut i)) as usize,
            "--money" => cfg.hh_cash = num(val(&mut i)) as i64,
            "--out" => cfg.out_dir = val(&mut i),
            "--watch-firm" => {
                let v = num(val(&mut i));
                cfg.watch_firm = if v < 0.0 { None } else { Some(v as usize) };
            }
            "--watch-home" | "--watch-hh" => {
                let v = num(val(&mut i));
                cfg.watch_home = if v < 0.0 { None } else { Some(v as usize) };
            }
            "--watch-person" => {
                let v = num(val(&mut i));
                cfg.watch_person = if v < 0.0 { None } else { Some(v as usize) };
            }
            "--replay-check" => replay = true,
            "--port" => port = num(val(&mut i)) as u16,
            "--quiet" => cfg.quiet = true,
            "--no-elections" => cfg.elections = false,
            "--no-browser" => {}
            "--election-every" => cfg.election_every_years = num(val(&mut i)) as u32,
            "--first-election" => cfg.first_election_year = num(val(&mut i)) as u32,
            "--no-credit" => cfg.credit_enabled = false,
            "--no-stock-market" => cfg.stock_trading = false,
            "--no-demographics" => cfg.demographics = false,
            "--bank-equity" => cfg.bank_equity = num(val(&mut i)) as i64,
            "--income-tax" | "--sales-tax" | "--luxury-tax" | "--dividend-tax" | "--benefit" | "--basic-income" | "--pension" | "--child-benefit" | "--inheritance-tax" | "--class-size" | "--teacher-pay" | "--min-wage" | "--surplus-dividend" | "--policy-rate" | "--print-rate" | "--ground-rent" => {
                let v = val(&mut i);
                if let Err(e) = cfg.policy.set(&a[2..], &v) {
                    eprintln!("{}", e);
                    usage();
                }
            }
            "--policy-at" => {
                let spec = val(&mut i);
                let (year, kv) = spec.split_once(':').unwrap_or_else(|| usage());
                let (k, v) = kv.split_once('=').unwrap_or_else(|| usage());
                let year: u32 = year.parse().unwrap_or_else(|_| usage());
                let mut probe = cfg.policy.clone();
                if let Err(e) = probe.set(k, v) {
                    eprintln!("{}", e);
                    usage();
                }
                cfg.schedule.push((year * MONTHS_PER_YEAR, k.to_string(), v.to_string()));
            }
            _ => usage(),
        }
        i += 1;
    }
    if let Some(y) = years {
        cfg.days = y * DAYS_PER_YEAR;
    }
    cfg.firm_cap = (cfg.n_firms * 4).max(64);
    (cfg, replay, port)
}

fn main() -> std::io::Result<()> {
    let all: Vec<String> = std::env::args().skip(1).collect();
    let mode = all.first().map(|s| s.as_str());
    if mode == Some("serve") || mode == Some("play") {
        let (mut cfg, _, port) = parse_args(&all[1..]);
        if mode == Some("play") && !all.iter().any(|a| a == "--no-browser") {
            let url = format!("http://127.0.0.1:{}/", port);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(600));
                open_browser(&url);
            });
        }
        cfg.quiet = true;
        cfg.days = u32::MAX;
        // the player starts with a centrist bundle and a full term before the first election
        if cfg.policy == government::Policy::default() {
            cfg.policy = politics::platform(0.5, cfg.init_wage as f64);
        }
        if cfg.first_election_year == 1 {
            cfg.first_election_year = cfg.election_every_years;
        }
        return server::serve(cfg, port);
    }
    let (cfg, replay, _) = parse_args(&all);
    let t0 = std::time::Instant::now();
    let mut world = World::new(cfg.clone())?;
    let minted = world.ledger.minted();
    let hash = world.run()?;
    let elapsed = t0.elapsed();

    let months = &world.stats.months;
    let last = months.last().expect("at least one month");
    let total_bankrupt: u32 = months.iter().map(|m| m.bankruptcies).sum();
    let total_entries: u32 = months.iter().map(|m| m.entries).sum();
    let total_exits: u32 = months.iter().map(|m| m.exits).sum();
    let last_year = &months[months.len().saturating_sub(MONTHS_PER_YEAR as usize)..];
    let mean = |f: &dyn Fn(&stats::MonthRow) -> f64| last_year.iter().map(f).sum::<f64>() / last_year.len() as f64;
    let sd = |f: &dyn Fn(&stats::MonthRow) -> f64, mu: f64| {
        (last_year.iter().map(|m| (f(m) - mu).powi(2)).sum::<f64>() / last_year.len() as f64).sqrt()
    };
    let p_mu = mean(&|m| m.price_index);
    let u_mu = mean(&|m| m.unemployment);
    let taxes: i64 = last_year.iter().map(|m| m.taxes).sum();
    let benefits: i64 = last_year.iter().map(|m| m.benefits).sum();
    let udiv: i64 = last_year.iter().map(|m| m.universal_dividend).sum();

    println!();
    println!("=== econsim run complete ===");
    println!("seed {}  days {}  households {}  firms {} -> {}  wall {:.2?}", cfg.seed, cfg.days, cfg.n_hh, cfg.n_firms, last.firms, elapsed);
    println!("policy at end: {}", world.gov.policy.describe());
    println!("money: initial {} cents, now {} cents in circulation (minted - burned = ledger total {}); {} transfers; conserved every tick: yes",
        minted, world.ledger.money_supply(), world.ledger.total(), world.ledger.transfers);
    println!("final year: cpi {:.1} (sd {:.2}), prices food {:.1} shelter {:.1} luxury {:.1}, avg pay {:.0}/wk, real wage {:.3}",
        p_mu, sd(&|m| m.price_index, p_mu), last.prices[0], last.prices[1], last.prices[2], last.wage, last.real_wage);
    println!("            unemployment {:.1}% (sd {:.1}; low-skill {:.1}%, high-skill {:.1}%), pay p10/p90 {}/{}, mean skill {:.3}",
        u_mu * 100.0, sd(&|m| m.unemployment, u_mu) * 100.0, last.unemp_low_skill * 100.0, last.unemp_high_skill * 100.0,
        last.pay_p10, last.pay_p90, last.mean_skill);
    println!("            gini {:.3}, top10% hold {:.0}%; sectors (firms/employed): food {}/{} shelter {}/{} luxury {}/{}",
        last.gini, last.top10_share * 100.0, last.firms_by_good[0], last.employed_by_good[0], last.firms_by_good[1],
        last.employed_by_good[1], last.firms_by_good[2], last.employed_by_good[2]);
    println!("            government: treasury {} cents; last year taxes {}, benefits {}, universal dividend {}", last.gov_cash, taxes, benefits, udiv);
    let defaults: u32 = months.iter().map(|m| m.defaults).sum();
    let lent: i64 = months.iter().map(|m| m.lent).sum();
    println!("            bank: loans outstanding {} (lent {} in total, {} defaults), equity {}, policy rate {:.1}%, loan rate {:.1}%, inflation {:.1}%",
        last.loans, lent, defaults, last.bank_cash, last.policy_rate * 100.0, last.loan_rate * 100.0, last.inflation * 100.0);
    println!("            debt {} cents ({} held by the bank) at {:.1}%; market cap {} cents, {} shares traded last month",
        last.gov_debt, last.bonds_bank, last.bond_rate * 100.0, last.market_cap, last.stock_volume);
    if cfg.elections {
        let shares: Vec<String> = politics::PARTIES.iter().zip(last.vote_shares.iter()).map(|((n, _), v)| format!("{} {:.0}%", n, v * 100.0)).collect();
        println!("            politics: {} elections; in office: {}; last vote {}; mean preference {:.2}",
            world.gov.elections_held, world.gov.incumbent.map(|k| politics::PARTIES[k].0).unwrap_or("-"), shares.join(", "), last.mean_pref);
    }
    if cfg.demographics {
        let births: u32 = months.iter().map(|m| m.births).sum();
        let deaths: u32 = months.iter().map(|m| m.deaths).sum();
        println!("            people: {} alive ({} minors, {} retirees), {} births and {} deaths over the run, mean age {:.1}, life expectancy at death {:.1}, dependency ratio {:.2}, parent-child skill correlation {:.2} over {} who came of age",
            last.population, last.minors, last.retirees, births, deaths, last.mean_age, last.life_expectancy, last.dependency, last.mobility_corr, world.mobility.len());
    }
    println!("firm dynamics: {} bankruptcies, {} voluntary exits, {} entries; {} events logged", total_bankrupt, total_exits, total_entries, world.events.count);
    println!("output hash {:016x}", hash);
    println!("            homes: {} ({} couples); schooling: public {} pupils / {} teachers (quality {:.2}), private {} pupils / {} teachers (quality {:.2}); public spend last month {}, tuition {}",
        last.homes, last.couples, last.pupils_public, last.teachers_public, last.public_quality, last.pupils_private, last.teachers_private,
        last.private_quality, last.education_spend, last.tuition);
    println!("files: {d}/daily.csv {d}/monthly.csv {d}/events.log {d}/watch.log {d}/people.csv {d}/homes.csv {d}/firms.csv", d = cfg.out_dir);

    if replay {
        let mut cfg2 = cfg.clone();
        cfg2.out_dir = format!("{}/replay", cfg.out_dir);
        cfg2.quiet = true;
        let mut w2 = World::new(cfg2)?;
        let hash2 = w2.run()?;
        if hash == hash2 {
            println!("replay check: identical output hash {:016x} -> deterministic", hash2);
        } else {
            println!("replay check FAILED: {:016x} != {:016x}", hash, hash2);
            std::process::exit(1);
        }
    }
    Ok(())
}
