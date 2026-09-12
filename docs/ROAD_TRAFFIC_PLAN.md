# Road and traffic implementation plan

Branch: `roads-and-agent-traffic`, based on `engine-foundation-and-physical-accounting`.
Read root STATUS.md, DIRECTION.md, DESIGN.md (Roads and traffic), and
IMPLEMENTATION_PLAN.md (C3) before executing. This plan implements C3a–c;
recipes, electricity, owned vehicles, and multi-day haulage remain outside scope.

## Decisions and sequencing

1. Adopt the documented 48 x 32 starter map and period-7 street grid now.
   Keep starter homes and firms adequately seated, preferentially road adjacent;
   verify default population and the previously failing 500-firm scenario.
   Add Road as zone 5, cost 60,000 cents, work 15, zero occupancy capacity.
   Use existing funding, materials, labor, cancellation, and demolition commands.
2. Put deterministic routing and traffic in a dedicated engine module. Road
   neighbors are orthogonal. Buildings enter/leave at adjacent road tiles.
   Break equal-cost choices by tile index. Cache routes by tile pair, refresh
   monthly using prior congestion and invalidate immediately on topology changes
   so demolition cannot leave phantom routes. Bound caches and avoid daily
   per-person Dijkstra searches. Walking uses Manhattan distance plus the stated
   8-tile penalty when a road route is unavailable; document this interpretation.
3. Generate commuting trips from current employed people (two directed trips
   each), shopping from actual distinct home/supplier purchases (one per pair),
   and construction freight from actual supplier/site delivery units (ceil /50).
   Aggregate by tile pair. Publish requested, routed, and off-network counts
   separately so walking does not silently disappear from commuter totals.
   Today's travel uses yesterday's flows; crossing cost is 1+0.15*(flow/400)^4.
   Recompute existing effective labor from current employment with this cost
   before construction/production; do not compound yesterday's productivity loss.
4. Require a connected supplier-to-site route before delivery. An unfinished road
   can receive deliveries from an adjacent completed road, allowing extension
   from the starter network. No disconnected new road magically delivers its own
   supplies. Report Access stalls distinctly from missing material/labor/budget;
   already delivered materials may still be worked. Preserve physical accounting.
   Add a modest documented road-access/congestion land-value term.
5. Add Road tools and traffic/connectivity views to both clients, with flow,
   capacity and access status in inspectors. Native rendering remains a state
   visualization; no decorative traffic simulation. Extend monthly CSV with
   clearly defined traffic totals/congestion diagnostics and keep existing tools
   compatible. Save authoritative trips, flows and route state; bump to version3.
6. Validate, review and document. Update root STATUS.md and both READMEs; update
   design claims only after verification. Root documents are outside Git, so keep
   this implementation plan within the crate for branch review. Do not merge,
   push, move the Git root, or commit unless separately requested.

## Acceptance tests

- Road command queues/funds/builds/demolishes with no premature connectivity.
- Orthogonal connectivity, deterministic ties, disconnected walking, route cache
  invalidation, and road extension from a completed network.
- Commute totals reflect current employment; layoffs halve an even workforce's
  next-day commuter count. Duplicate purchases yield one shopping trip per pair.
- Congestion lowers effective labor/output; an alternate corridor restores them
  after route refresh. Verify lag semantics without feeding back within a tick.
- Disconnected construction stalls with Access and resumes upon connection;
  money and goods reconcile throughout delivery, completion and cancellation.
- Save/resume preserves routes/flows and deterministic continuation; old saves
  reject clearly. Test both feature sets and compile desktop release.
- Four-year deterministic replay; save/resume across road changes/month boundary.
- Benchmark default 1,000 households for 20 years against documented 4.4-second
  gate. Report actual measurement and any failed gate honestly; optimize measured
  routing bottlenecks rather than weakening economic behavior.
- Smoke-test client data and controls where environment allows. Distinguish
  compiled UI from runtime verification.

## Work ownership

New road-system agent owns engine, command, construction, routing, stats,
client implementation and focused tests. Root owns independent review,
end-to-end acceptance checks and final documentation integration. Agent sends
API and milestone updates before requesting root review. Work sequentially by
stage, keeping the branch buildable at each integration point.

## Completion notes — 2026-09-12

C3a–c implemented and verified on this branch, uncommitted. Initial agent work
was completed by root with a traffic-integrity review. Starter lots now mix
homes/firms near the center and zone only requested capacity plus 20%; commuting
uses 0.006 per congested cost unit, calibrated from the initial 0.015 that caused
startup collapse. Capacity and congestion curve stay unchanged.

37 release tests pass with desktop (36 without). Four-year replay and two-plus-
two-year save/resume agree at `2896c4b7646b124f`. Default 20-year run is 4.246 s,
within 4.4 s, hash `fef94464a321933c`; startup unemployment peak 21.15%, final-year
mean 10.42%. Native overlays/persistence and HTTP construction/access recovery
smoke checks pass. Expanded 5,000-household/500-firm initialization passes.

Save format 3 retains paths/frozen weights/flows/trips but rebuilds large derived
indexes/search trees; a default two-year snapshot is 1,034,362 bytes. Trips are
daily aggregate assignment on final topology, not individual timed journeys.
No freight delay/tonnage cap, transit, vehicle ownership, or road maintenance
is claimed. Root STATUS.md carries current evidence and limits.
