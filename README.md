# economy-simulator

A city simulator in which the whole economy is agent-based: people act on
drives and personality, found and run firms, and a government legislates by
moving the same levers agents already respond to. This repository is at the
**economy prototype with an optional native 3D city view** stage. Done so far: M0 (price stability, money
conservation), M1 (three goods, heterogeneous firms, entry and bankruptcy),
M2 (labor skills, skill-based pay, structural unemployment), M4 (a government
with taxes, transfers and a minimum wage as levers, chosen by elections
between three parties), M5 (a bank that creates money by lending to firms
and buying government bonds, a bond market, a stock market) and M6 (people
live in one- or two-adult homes, have children who go to public or private
schools staffed by teachers hired on the labor market, come of age with a
skill shaped by that schooling and by their parents, partner and separate,
retire, die and leave estates). M3 space includes zoning, capacity, land values, commuting, and resource-constrained construction.

    econsim/           shared Rust engine, persistence, optional native graphics
    tools/dashboard.py builds a chart dashboard (HTML) from a run's CSV output
    DESIGN.md          economic model and historical findings
    DIRECTION.md       proposed government/resource-planning game direction
    IMPLEMENTATION_PLAN.md  delivery milestones and current implementation status

## Native 3D desktop

From the repository root:

```sh
cd econsim
cargo run --release --features desktop -- desktop
```

This opens a native graphics window and runs the Rust economy directly, without
an HTTP server or browser. The optional Macroquad renderer draws a perspective
city with occupancy-dependent building heights, windows, schools, and trees.
It needs a working desktop graphics driver; the first build downloads Cargo
dependencies. Headless and browser builds do not enable the graphics feature.

- Right-drag to orbit, scroll to zoom, WASD to pan, Home to reset the camera.
- Select homes, businesses, schools, parks, or demolition and click a lot.
  Preview outlines turn red for occupied lots or insufficient funds.
- Orders reserve a budget at month end, then become multi-day projects.
  Use **Month +** (N) while paused, then advance time to perform construction.
- Space pauses/resumes; choose 1, 7, or 28 simulation days per second.
- Hover over a lot to inspect occupancy, land value, or construction progress,
  delivered materials, spending, and the reason work is stalled.
- **X** over a site cancels its queued order or active project. Cancellation
  refunds unspent escrow; purchased materials and completed work are sunk costs.
- **F5 / Save** writes a resumable game; **F9 / Load** restores it and pauses.
  Escape saves and exits. Default save: `out/city.econsave`; use `--save FILE`
  to choose another path. Native loads write logs into fresh `resume-N` folders.

The simulation still uses discrete lots, and the detailed policy sliders, charts,
and election dashboard remain in the browser interface. Roads/freight, electricity,
and terrain editing remain future milestones. New-game command-line policy
options also work in native mode.
For the graphics API, see [Macroquad's 3D examples](https://macroquad.rs/examples/).

## Play in the browser

    cd econsim
    cargo run --release -- play             # opens the game in a browser window

Install the native 3D launcher on Linux with `desktop/install.sh`, after which
Econsim appears in the application menu. For the browser interface, `play` starts the simulator and opens the
page as an app window in Chromium or Chrome if present, else Firefox, else
the default browser; `serve` does the same without opening anything (visit
http://127.0.0.1:8080). The browser client uses a local Rust HTTP server and one
HTML page. All clients share the engine library; serde/bincode provide snapshot
persistence and native graphics are an optional dependency.

You are the government and the city planner. The page shows the economy live with a clock, nine
stat tiles, charts, an event feed and a poll; the left panel has every lever
as a slider (taxes, benefits, pension, basic income, child benefit, minimum
wage, public school class size and teacher pay, the surplus rule, the
central bank's interest rate, a money-printing rate and the ground rent, plus
one-off print and burn buttons). Apply changes at any time.

The region is a 48 x 32 grid with streets every seven tiles. Homes and firms
start on mixed central frontage, zoned for the requested population plus 20%
headroom; distant lots remain available for expansion. Pick a tool (residential,
business, school, park, road, demolish) and click a tile. At month end, affordable orders reserve their
budgets in escrow and clear their sites. Construction then buys finite inventory
and hires a share of existing building-trade labor, reducing ordinary production.
Projects take at least four construction days; buildings provide no capacity
until complete. Material, labor, budget, and road-access shortages delay progress. Unspent
escrow returns to the treasury on completion or cancellation. The browser shows
project progress, cancellation buttons, command feedback, and a **Save game** button.
Demolishing an unoccupied lot still completes at month end without construction.

For this first resource-planning slice, building materials use existing shelter
inventory; dedicated material goods and durable housing services are the next
modeling step. Exhausted project budgets require cancellation and replanning.

Roads cost **$600** and require materials and 15 work units. Roads connect
orthogonally. Deliveries need a continuous road route from a supplier to a road
adjacent to the site. Extend roads outward from completed streets; disconnected
sites report **road access missing**. Already delivered materials remain usable.

Traffic comes from actual activity: two commute trips per employed person per
day, one trip per home/supplier pair that made a purchase, and one freight trip
per 50 delivered units (rounded up per delivery). Capacity is 400 trips per road
tile per day; congestion reduces effective labor and output. Routes refresh
monthly and after road edits; travel costs use yesterday's flows. People without
a route walk with an eight-tile penalty; freight requires a route. Deliveries
complete within the day—vehicles and multi-day haulage are not modeled.

Press **T** in the native client, or use the browser's view selector, to switch
between city, traffic, and road-adjacency views. Inspect lots for flow/capacity,
access, and construction stall reasons. Green adjacency does not guarantee a
route to a particular supplier; the site's access stall checks that route.
`monthly.csv` includes requested commute/shopping/freight trips, routed and
off-network totals, peak tile flow, and average delay across used roads.

Homes need residential room (20 per tile),
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

## Save and resume

From `econsim/`:

```sh
cargo run --release -- --days 336 --out out --save out/city.econsave
cargo run --release -- --load out/city.econsave --days 28 --out continued
cargo run --release --features desktop -- desktop --load out/city.econsave --out desktop-resumed
```

`--out` on load must be a **new directory with an existing parent**. This keeps
previous run outputs intact. A loaded headless run advances the requested number
of **additional** days (or years); saved simulation settings are restored, while
`--out`, duration, and `--quiet` are taken from the command line. `--save FILE`
writes after a headless run or selects the native/browser Save destination.
Use the browser Save button before stopping its server; it does not autosave on
Ctrl-C. Resume a browser game with `serve --load FILE --out NEW_DIRECTORY`.

Saves include RNG streams, policy schedules, construction progress and escrow,
pending commands, agents, financial state, and statistical history. Their version
and checksum are validated; CSV/event files in the new directory contain the
continuation, while monthly history and the cumulative output hash are preserved.
Snapshot format **3** includes roads, trips, flows, and monthly route choices.
Versions 1 and 2 are rejected; there is no backward-version migration support.
Derived route lookup indexes and search trees are rebuilt on load.

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
- **Producer goods reconcile daily.** Opening stock plus production equals
  closing stock plus household consumption, construction allocation, and stock
  discarded when firms close. The assertion runs in release builds; the engine's
  `physical_goods_balance()` report records each flow for the last completed day.
  Education seats and completed buildings are outside this unit accounting.
  Construction materials count as irreversibly allocated when delivered.
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
