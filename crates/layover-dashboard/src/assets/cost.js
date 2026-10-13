// Cost: what the factory spent, over windows you can reason about, day by day, by agent, model,
// workflow and chain — and how fast it is going through the Reserve.
//
// Uses app.js's helpers (`get`, `el`, `$`, `money`, `spend`, `provenance`, `scope`, `scoped`,
// `WINDOWS`, `routeChanged`) and chain.js's `openChain`.

// The windows worth a headline figure. Not all of them: seven numbers is a wall, and these four
// are the ones that answer "is today unusual" and "where is the month going".
const HEADLINES = ["today", "last_7d", "last_30d", "month_to_date"];
const costView = { selected: "last_30d" };
// How far back the Reserve's pace is read: long enough to smooth over one expensive run, short
// enough to notice a factory that has just started running hot.
const PACE_HOURS = 3;

// The Cost tab's address carries the window being looked at; see router.js.
routeExtras.cost = (params) => {
  if (params) {
    const wanted = params.get("window");
    costView.selected = HEADLINES.includes(wanted) ? wanted : "last_30d";
    return undefined;
  }
  return { window: costView.selected === "last_30d" ? "" : costView.selected };
};

function costCard(window, label, report, selected) {
  const card = el("div", `card${selected ? " selected" : ""}`);
  const button = el("button");
  button.append(el("span", "k", label), el("span", "v", spend(report.total)));

  const [tone, note] = provenance(report.total);
  if (note) button.append(el("span", `n ${tone}`, note));

  button.addEventListener("click", () => {
    costView.selected = window;
    routeChanged();
    loadCost();
  });
  card.append(button);
  return card;
}

function costRows(table, buckets) {
  const body = table.querySelector("tbody");
  body.replaceChildren();

  if (buckets.length === 0) {
    const row = el("tr");
    row.append(el("td", "empty", "Nothing recorded."));
    body.append(row);
    return;
  }

  for (const bucket of buckets) {
    const row = el("tr");
    const [tone] = provenance(bucket.summary);
    row.append(
      el("td", "", bucket.name),
      el("td", "num", `${bucket.summary.runs}`),
      el("td", `num ${tone}`, spend(bucket.summary)),
    );
    body.append(row);
  }
}

/// A day as a chart labels it: "Mon 13".
function dayLabel(day, long) {
  const [year, month, date] = day.split("-").map(Number);
  return new Date(year, month - 1, date).toLocaleDateString([], {
    weekday: "short",
    day: "numeric",
    month: long ? "short" : undefined,
  });
}

/// Spend day by day, as bars. Every day of the window is there, the quiet ones too, so the gaps
/// read as gaps. A day that includes runs reporting no cost is a floor, and its bar says so.
function costDays(report) {
  const host = $("#cost-days");
  const days = report.by_day ?? [];
  host.hidden = days.length < 3;
  if (host.hidden) return;

  const peak = Math.max(...days.map((day) => day.summary.usd));
  const total = days.reduce((sum, day) => sum + day.summary.usd, 0);
  const busiest = days.find((day) => day.summary.usd === peak);
  const where = report.day_zone ? `days in ${report.day_zone}` : "days where the Tower is";
  $("#cost-days-what").textContent = peak > 0
    ? `${where} · peak ${money(peak)} on ${dayLabel(busiest.day, true)} · ${money(total / days.length)} a day on average`
    : `${where} · nothing spent`;

  const chart = $("#cost-chart");
  const labels = $("#cost-chart-days");
  chart.replaceChildren();
  labels.replaceChildren();
  // Few enough labels to read: the first, the last, and evenly between.
  const every = Math.max(1, Math.ceil(days.length / 10));
  days.forEach((day, index) => {
    const { summary } = day;
    const column = el("div", "day");
    const bar = el("div", `bar${summary.unreported_runs > 0 ? " floor" : ""}`);
    bar.style.height = peak > 0 ? `${(summary.usd / peak) * 100}%` : "0";
    column.title = [
      dayLabel(day.day, true),
      `${spend(summary)} over ${summary.runs} run(s)`,
      summary.unreported_runs > 0 ? `${summary.unreported_runs} reported no cost, so this is a floor` : "",
      index === days.length - 1 ? "today, so far" : index === 0 ? "the window starts part-way through this day" : "",
    ]
      .filter(Boolean)
      .join("\n");
    column.append(bar);
    chart.append(column);
    const shown = index === 0 || index === days.length - 1 || index % every === 0;
    labels.append(el("span", "", shown ? dayLabel(day.day) : ""));
  });
}

/// The chains that cost the most in the window, so "what was expensive" has an answer one level
/// below the agent: this piece of work, rather than the reviewer in general.
async function costChains(window) {
  const body = $("#cost-chains tbody");
  try {
    const { itineraries } = await get(`/itineraries?${new URLSearchParams({ window })}`);
    const top = itineraries
      .filter((chain) => !scope() || chain.pipeline === scope())
      .filter((chain) => chain.usd > 0)
      .sort((a, b) => b.usd - a.usd)
      .slice(0, 10);
    body.replaceChildren();
    if (top.length === 0) {
      const row = el("tr");
      row.append(el("td", "empty", "Nothing recorded."));
      body.append(row);
      return;
    }
    for (const chain of top) {
      const row = el("tr", "readable opens");
      const name = el("td");
      name.append(
        chain.name ? el("b", "", chain.name) : el("code", "", `…${chain.itinerary_id.slice(-6)}`),
        el("span", "muted", ` ${chain.pipeline ?? ""}`),
      );
      row.append(
        name,
        el("td", "num", `${chain.runs}`),
        el("td", `num${chain.measured === false ? " warn" : ""}`, chain.measured === false ? `${money(chain.usd)}+` : money(chain.usd)),
      );
      row.title = `${chain.itinerary_id} — open this chain`;
      row.addEventListener("click", () => openChain(chain.itinerary_id));
      body.append(row);
    }
  } catch (error) {
    const row = el("tr");
    row.append(el("td", "empty", `Could not read chains: ${error.message}`));
    body.replaceChildren(row);
  }
}

/// Draws the Reserve: the factory's own spending ceiling over its own rolling window.
///
/// Deliberately outside the window cards and outside the workflow scope. The cards answer "what
/// did this cost"; the Reserve answers "what may still be spent", over hours it chose rather than
/// the period being browsed, across every workflow rather than the selected one. Drawing it
/// alongside figures that narrow would invite reading it as though it narrowed too.
function showReserve(reserve) {
  const host = $("#reserve");
  if (!reserve || reserve.cap_usd === null || reserve.cap_usd === undefined) {
    // No cap configured means unlimited, and a meter with no ceiling is a decoration.
    host.hidden = true;
    return;
  }

  const spent = reserve.spent_usd;
  const cap = reserve.cap_usd;
  const share = cap > 0 ? Math.min(spent / cap, 1) : 0;

  $("#reserve-what").textContent =
    `the whole factory, rolling ${reserve.window_hours}h \u2014 not narrowed by workflow`;
  const fill = $("#reserve-fill");
  fill.style.width = `${(share * 100).toFixed(1)}%`;
  fill.className = reserve.exhausted ? "bad" : share > 0.8 ? "warn" : "";

  $("#reserve-detail").textContent = reserve.exhausted
    ? `${money(spent)} of ${money(cap)} spent. Exhausted \u2014 new work is refused until the window rolls forward.`
    : `${money(spent)} of ${money(cap)} spent, ${money(reserve.remaining_usd ?? 0)} left before work is refused.`;

  host.hidden = false;
  reservePace(reserve);
}

/// How fast the Reserve is going: the last few hours' spend, as it would be over the Reserve's
/// whole window. A meter says how full it is; this says whether it is filling faster than it can
/// hold. Read from every workflow's runs, as the Reserve is.
async function reservePace(reserve) {
  const line = $("#reserve-pace");
  line.textContent = "";
  try {
    const { runs } = await get("/runs?window=last_24h&limit=2000");
    const since = Date.now() - PACE_HOURS * 3600 * 1000;
    const recent = runs.filter(
      (run) => run.finished_at && new Date(run.finished_at).getTime() >= since,
    );
    const spent = recent.reduce((sum, run) => sum + (run.cost_usd ?? 0), 0);
    const floor = recent.some((run) => run.cost_usd === null) ? "+" : "";
    const perHour = spent / PACE_HOURS;
    const whole = perHour * reserve.window_hours;
    const share = Math.round((whole / reserve.cap_usd) * 100);
    if (spent === 0) {
      line.textContent = `Nothing spent in the last ${PACE_HOURS} hours.`;
      line.className = "hint";
      return;
    }
    const pace = `At the pace of the last ${PACE_HOURS} hours — ${money(spent)}${floor} — a full ${reserve.window_hours}h would spend about ${money(whole)}${floor}, ${share}% of the cap.`;
    if (whole > reserve.cap_usd && !reserve.exhausted) {
      const left = (reserve.remaining_usd ?? 0) / perHour;
      line.textContent = `${pace} That is faster than the cap allows: the Reserve would run out in about ${left < 1 ? "under an hour" : `${Math.round(left)} h`}, later if earlier spend rolls out of the window first.`;
      line.className = "caveat";
    } else {
      line.textContent = pace;
      line.className = "hint";
    }
  } catch {
    line.textContent = "";
  }
}

async function loadCost() {
  const selected = costView.selected;
  const cards = $("#cost-cards");
  const caveat = $("#cost-caveat");
  const note = $("#cost-scope");

  try {
    const reports = await Promise.all(
      HEADLINES.map(async (window) => [
        window,
        await get(`/costs?${scoped(new URLSearchParams({ window }))}`),
      ]),
    );

    cards.replaceChildren();
    for (const [window, report] of reports) {
      const label = WINDOWS.find(([slug]) => slug === window)[1];
      cards.append(costCard(window, label, report, window === selected));
    }

    const detail = reports.find(([window]) => window === selected)?.[1]
      ?? (await get(`/costs?${scoped(new URLSearchParams({ window: selected }))}`));

    costDays(detail);
    costRows($("#cost-agents"), detail.by_agent);
    costRows($("#cost-models"), detail.by_model);
    costRows($("#cost-pipelines"), detail.by_pipeline);
    costChains(selected);

    // The Reserve caps the factory, not a workflow. Narrowing the page must not quietly change
    // what the Reserve is comparing, so it is drawn separately and says which it is showing.
    showReserve(detail.reserve);
    note.textContent = scope()
      ? `Totals and breakdowns are for ${scope()}.`
      : "";
    note.hidden = !scope();

    const notes = [];
    if (detail.span.calendar && detail.span.zone) {
      notes.push(`"${detail.span.label}" begins at midnight in ${detail.span.zone}.`);
    }
    if (detail.span.truncated) {
      notes.push("This window reaches past the 90 days kept, so the total is a lower bound.");
    }
    // The `+` is everywhere a total includes a run that reported nothing; say once what it means.
    if (reports.some(([, report]) => report.total.unreported_runs > 0)) {
      notes.push(
        "A figure ending in + is a floor: some of its runs reported no cost, so the real total is at least that.",
      );
    }
    caveat.textContent = notes.join(" ");
    caveat.hidden = notes.length === 0;
  } catch (error) {
    cards.replaceChildren(el("p", "empty", `Could not read costs: ${error.message}`));
    caveat.hidden = true;
    note.hidden = true;
  }
}
