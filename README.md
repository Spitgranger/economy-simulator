# economy-simulator

A city simulator in which the whole economy is agent-based: people act on
drives and personality, found and run firms, and a government legislates by
moving the same levers agents already respond to. This repository is at the
**headless proof-of-concept** stage. Done so far: M0 (price stability, money
conservation), M1 (three goods, heterogeneous firms, entry and bankruptcy),
M2 (labor skills, skill-based pay, structural unemployment), M4 (a government
with taxes, transfers and a minimum wage as levers, chosen by elections
between three parties), M5 (a bank that creates money by lending to firms
and buying government bonds, a bond market, a stock market) and M6 (people
live in one- or two-adult homes, have children who go to public or private
schools staffed by teachers hired on the labor market, come of age with a
skill shaped by that schooling and by their parents, partner and separate,
retire, die and leave estates). M3 space is the missing rung.

    econsim/           Rust crate, zero dependencies
    tools/dashboard.py builds a chart dashboard (HTML) from a run's CSV output
    DESIGN.md          design decisions, milestone ladder, findings so far

## Play it

    cd econsim
    cargo run --release -- play             # opens the game in a browser window

Or install a launcher on Linux with `desktop/install.sh`, after which Econsim
appears in the application menu. `play` starts the simulator and opens the
page as an app window in Chromium or Chrome if present, else Firefox, else
the default browser; `serve` does the same without opening anything (visit
http://127.0.0.1:8080). The engine and the interface are dependency-free
Rust plus one HTML page, so this is the whole desktop stack: no Electron, no
webview toolchain to build.

You are the government and the city planner. The page shows the economy live with a clock, nine
stat tiles, charts, an event feed and a poll; the left panel has every lever
as a slider (taxes, benefits, pension, basic income, child benefit, minimum
wage, public school class size and teacher pay, the surplus rule, the
central bank's interest rate, a money-printing rate and the ground rent, plus
one-off print and burn buttons). Apply changes at any time.

The city is a 24 x 16 grid. Pick a tool (residential, business, school,
park, demolish) and click a tile; the cost leaves the treasury at month end
and goes to the building trade. Homes need residential room (20 per tile),
firms need business lots (6 per tile), pupils need school room (250 per
tile). Newcomers arrive while there is housing and work; a full city means
unhoused homes and blocked founders. Land value follows occupancy and what is
nearby; ground rent on it is treasury revenue; commuting distance lowers what
a worker produces, so firms prefer people who live close. Speed buttons run the world at 1 day, 1 week, 1 month
or 1 year per real second, or as fast as the machine allows (about 8 years a
second); pause and step by a day or a month. Every four years the voters
choose between your bundle and the three parties, judging the incumbent on
their own real income; lose, and the winner governs for a term while your
sliders become your platform for the next election. `serve` accepts the same
options as a headless run (`--seed`, `--households`, `--no-credit`, ...) plus
`--port`. The world is written to `out/` once a year as in headless mode.

## Run it headless

    cd econsim
    cargo run --release -- --years 20 --replay-check          # laissez-faire baseline
    python3 ../tools/dashboard.py out --title "Baseline"      # writes out/dashboard.html

Government levers are flags, each routed through a mechanism agents already use:

    --income-tax 0.2        withheld from wages at payroll
    --sales-tax 0.05        added to purchase prices (all goods)
    --luxury-tax 0.5        sales tax on the luxury good only
    --dividend-tax 0.25     withheld when firms pay owners
    --benefit 0.5           weekly unemployment benefit, fraction of the average wage
    --min-wage 2200         floor on gross weekly pay, cents
    --surplus-dividend 1    share of treasury above one month of benefits returned
                            equally to all households each month (default 1)
    --basic-income 0.1      weekly payment to every adult, fraction of the average wage
    --pension 0.3           weekly payment to every retiree (65+), fraction of the average wage
    --child-benefit 0.1     weekly payment per minor to the parent; also public education quality
    --inheritance-tax 0.4   share of a cash estate taken at death
    --class-size 25         public school target pupils per teacher (0 = no public school)
    --teacher-pay 1.1       public teacher pay as a multiple of the average wage
    --policy-rate 0.03      central bank rate set by hand (annual); default auto = Taylor rule
    --print-rate 0.01       new money minted into the treasury each month, share of all money
    --ground-rent 0.01      monthly ground rent as a share of land value (homes x1, firms x2)
    --policy-at 5:benefit=0.5   change any lever at the start of a year (repeatable)

Politics and finance:

    --no-elections          keep the levers as set; by default households vote every
                            4 years from year 1 for Market / Centre / Labour platforms
    --election-every N      years between elections
    --first-election N      year of the first election
    --no-credit             the bank neither lends nor buys bonds (fixed money stock)
    --no-stock-market       shares are quoted but never traded
    --no-demographics       nobody ages, is born or dies (everyone is 30, one per home)
    --watch-home N          trace home N in watch.log; --watch-person N traces a person
    --bank-equity CENTS     bank equity target; lending capacity is 10x (default 1000000)

Examples:

    # default: elections, credit, bonds and shares all on
    cargo run --release -- --years 20 --out out_full
    # a welfare state introduced in year 5, no elections
    cargo run --release -- --years 20 --out out_reform --no-elections \
      --policy-at 5:income-tax=0.2 --policy-at 5:benefit=0.5 --policy-at 5:dividend-tax=0.25
    # a debt trap: generous transfers, almost no tax, bonds fill the gap
    cargo run --release -- --years 20 --out out_deficit --no-elections \
      --benefit 0.5 --basic-income 0.08 --income-tax 0.05
    # three generations under elected governments (about 7 seconds)
    cargo run --release -- --years 60 --out out_demo
    # no public school at all: a private school market fills the gap for those who can pay
    cargo run --release -- --years 60 --out out_noschool --no-elections --class-size 0

Other options: `--seed`, `--years`/`--days`, `--households`, `--firms`,
`--workers`, `--money`, `--watch-firm N`, `--watch-hh N`, `--replay-check`,
`--quiet`. A 20-year run of 1000 households and three sectors takes 3 to 5 seconds.

Output in the `--out` directory:

| file | content |
|---|---|
| `daily.csv` | CPI, price per good, wage, employment, output, sales, unmet demand, inventory, cash held by households / firms / treasury / bank, money supply, loans, public debt, active firms |
| `monthly.csv` | the above plus real wage, unemployment by skill group, employment and firms per sector, markup, Gini, top-decile share, taxes, benefits, universal dividend, pay percentiles, mean skill, entries, bankruptcies, exits, dividends, hires/fires/quits, lending, defaults, write-offs, policy / loan / bond rates, inflation, deposit interest, bonds held by the bank, coupons, market cap, shares traded, mean political preference, incumbent, vote shares |
| `events.log` | firm births and deaths with reasons; policy changes; election results; loan defaults; bond auctions that failed; debt-brake switches; bank bailouts |
| `watch.log` | decision trace of one firm and one household (`--watch-firm`, `--watch-hh`) |
| `people.csv`, `homes.csv`, `firms.csv` | end-of-run snapshot of every living person, every home (with its tile) and every active firm (including the public school) |

## What holds by construction

- **Money is conserved.** All money is integer cents in one double-entry
  ledger (`ledger.rs`). It is created only by `mint` (the initial endowment,
  and the bank when it lends or buys bonds) and destroyed only by `burn`
  (principal repaid, or bank equity absorbing a default). Everything else is
  a transfer that panics on overdraft. After every daily tick the sum of all
  balances is asserted equal to minted minus burned, and monthly the bond
  books are asserted consistent (bonds held equals debt equals issue faces).
- **Deterministic.** Five independent xoshiro256** streams (households, firms,
  goods market, labor market, entry) seeded from one seed. `--replay-check`
  runs the world twice and compares a hash of all daily output.
- **Structure of arrays.** Households and firms are indices into column
  vectors; the tick loop allocates nothing beyond a few reused buffers.
- **Staggered ticks.** Production and consumption daily; payroll (with income
  tax), benefits, basic income, layoffs and job search weekly; policy schedule,
  elections, loan service and borrowing, price/wage/hiring rules, dividends to
  shareholders, skill and preference drift, the stock market session, the
  bond auction and debt service, the bank's rate setting, entry and exit
  monthly; school enrollment and learning, aging, births, coming of age,
  partnering, separation, retirement and death monthly too; the stock market
  clears weekly. A week is 7 days, a month 4 weeks, a year 12 months.

See `DESIGN.md` for the agent rules, the results so far (price stability,
cycles, what each policy lever does) and the lessons from the collapses it
took to get there.
