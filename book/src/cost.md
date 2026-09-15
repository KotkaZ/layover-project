# Cost

Layover spends real money with nobody watching, so it is deliberately opinionated about what a
cost figure means.

## Two budgets, not one

| | Bounds | Resets |
|---|---|---|
| **Fuel** (`[defaults] fuel_usd`) | One itinerary | Every new trigger |
| **Reserve** (`[reserve] fuel_usd`) | The whole factory | Rolls continuously |

You need both, and the reason is arithmetic rather than taste. A scheduled pipeline mints a
**fresh itinerary with a fresh Fuel budget on every tick**:

```text
hourly pipeline × $20 Fuel = $480 a day
```

Every one of those 24 chains sits perfectly inside its rail. Fuel is working exactly as designed
and the total still ran away. Only the Reserve sees it.

```toml
[reserve]
fuel_usd     = 120.00   # at most this much...
window_hours = 24       # ...in any rolling 24 hours
```

`layover validate` warns when a factory has a scheduled pipeline and **no** Reserve, and when a
Reserve is too small to fund even one run of a pipeline. It does *not* warn merely because the
Reserve is below the theoretical worst case — capping below worst case is the entire reason to
have a cap, and reaching it pauses the factory rather than breaking it.

### Why the window rolls instead of resetting at midnight

A daily cap is worse twice over:

- **Midnight doubles it.** Spend the cap at 23:59 and the bucket resets a minute later, so "$50 a
  day" permits $100 in two minutes.
- **A day needs a timezone.** Bucket the gate in UTC and the ledger in local time and, between
  local midnight and the offset, the gate reads the wrong day's total and lets spending through.
  That is a real bug from a real system, not a hypothetical.

"At most $120 in any rolling 24 hours" has no midnight, no timezone, and no daylight-saving edge.

## Where a number came from is part of the number

Every run's cost carries a `CostSource`:

| Source | Meaning |
|---|---|
| `reported` | The runner said so. The only kind worth billing against. |
| `rate_card` | Layover derived it from token counts and published prices. An estimate. |
| `unreported` | The runner said nothing. The figure is zero and means nothing. |

**A total reports the weakest source that fed it.** Ninety-nine measured runs and one estimate
make an estimate. This looks pedantic until you see what the alternative costs: a system that
priced its runs from a hand-maintained table ran **2.7× over actual** — billing one model at `$75`
per million output tokens where the provider charged `$25` — and nothing in its totals said "this
is a guess".

`measured_share` tells you the ratio directly. If it is below 1.0, your remaining budget is an
upper bound, not a measurement.

## Rate cards

Optional, and only ever a fallback for a runner that reports tokens but not dollars.

```toml
[rates.claude-opus-4]
input_usd       = 5.00
output_usd      = 25.00
cache_read_usd  = 0.50
cache_write_usd = 6.25
```

Four rates rather than one because providers price cached tokens far below fresh input — often ten
to one — and a single blended rate is wrong by whatever the cache hit rate happened to be.

**Layover ships no rate card.** Prices change, differ per provider and per context tier, and a
stale table baked into a release is exactly how a cost estimate drifts by a factor of two without
anyone noticing. An unknown model produces `unreported`, never a flattering zero.

## Reading the bill

```sh
curl localhost:7878/costs
curl 'localhost:7878/costs?window_hours=24'
```

```json
{
  "total": {
    "runs": 31, "usd": 18.40,
    "unreported_runs": 2, "estimated_runs": 0,
    "confidence": "unreported", "measured_share": 0.935
  },
  "by_agent": [ { "name": "developer", "summary": { "usd": 11.20, "runs": 9 } } ],
  "by_model": [ { "name": "claude-opus-5", "summary": { "usd": 14.00, "runs": 11 } } ],
  "reserve": { "cap_usd": 120.0, "spent_usd": 18.40, "remaining_usd": 101.60, "exhausted": false }
}
```

That `"confidence": "unreported"` with 2 of 31 runs unmetered is the number that matters: the bill
is a **lower bound**, and whichever runner is silent needs looking at.

## When a rail bites

| Denial | Means |
|---|---|
| `HopsExhausted` | The chain hit its depth limit. |
| `FuelExhausted` | This itinerary spent its budget. |
| `RunCapReached` | This itinerary hit `max_runs` — the backstop that holds when cost reporting does not. |
| `ReserveExhausted` | The **factory** spent its window budget. This itinerary may have Fuel to spare. |

Ground Stop is separate and absolute: it is a file on disk, so it survives a Tower crash and can
be set by hand when nothing else is responding.
