// What will start next, and when: work queued for a slot, every scheduled tick in the window, the
// layovers waiting to be picked up, and the ticks that came due and were skipped.
//
// The times come from the Tower that keeps the clock (`GET /upcoming`), not from this page: an
// `every` schedule is counted from when that Tower started, so a page that worked the times out
// for itself would show a timetable that looks exact and is not.
//
// Separate from app.js, which is already long; it uses that file's helpers (`get`, `send`, `el`,
// `when`, `scope`, `openChain`, `pipelinesByName`), all of which exist by the time anything here is
// called.

const upcomingView = { timer: null };

/// Called whenever the visible view changes. Kept current while visible, because "in 3 min" goes
/// stale, and polling stops the moment the view is left.
function watchUpcoming(visible) {
  clearInterval(upcomingView.timer);
  upcomingView.timer = null;
  if (!visible) return;
  loadUpcoming();
  upcomingView.timer = setInterval(() => {
    if (!document.hidden) loadUpcoming();
  }, 15000);
}

function startOfDay(date) {
  const day = new Date(date);
  day.setHours(0, 0, 0, 0);
  return day.getTime();
}

/// When, in the reader's own time zone: "14:00", "tomorrow 09:30", "Thu 9 Oct 14:00".
function clockTime(iso) {
  const at = new Date(iso);
  const time = at.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const days = Math.round((startOfDay(at) - startOfDay(new Date())) / 86400000);
  if (days === 0) return time;
  if (days === 1) return `tomorrow ${time}`;
  if (days === -1) return `yesterday ${time}`;
  return `${at.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })} ${time}`;
}

/// How far off, measured from `now`: "in 23 min", "in 2 h 5 min", "in 3 d", or "due now".
function fromNow(iso, now) {
  const seconds = (new Date(iso) - now) / 1000;
  if (seconds <= 0) return "due now";
  if (seconds < 60) return "in under a minute";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `in ${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return minutes % 60 ? `in ${hours} h ${minutes % 60} min` : `in ${hours} h`;
  return `in ${Math.round(hours / 24)} d`;
}

/// The time of day alone, for a row under a heading that already says which day.
function timeOfDay(iso) {
  return new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/// The day a tick falls on, as a heading: "Today", "Tomorrow", "Thursday 9 October".
function dayHeading(iso) {
  const days = Math.round((startOfDay(iso) - startOfDay(new Date())) / 86400000);
  if (days <= 0) return "Today";
  if (days === 1) return "Tomorrow";
  return new Date(iso).toLocaleDateString([], { weekday: "long", day: "numeric", month: "long" });
}

function chainLink(id) {
  const open = el("button", "link", "");
  open.type = "button";
  open.append(el("code", "", id));
  open.title = "See this chain whole";
  open.addEventListener("click", () => openChain(id));
  return open;
}

async function loadUpcoming() {
  const hours = $("#upcoming-hours").value;
  try {
    const [upcoming, queue] = await Promise.all([get(`/upcoming?hours=${hours}`), get("/flights")]);
    const now = new Date(upcoming.now);
    showClock(upcoming);
    showQueue(queue);
    showFires(upcoming, now, hours);
    showLayovers(upcoming, now);
    showSkips(upcoming);
  } catch (error) {
    $("#upcoming-clock").textContent = `Could not read what is coming: ${error.message}`;
  }
}

function showClock(upcoming) {
  $("#upcoming-clock").textContent = upcoming.clock
    ? `Times are from the clock of ${upcoming.clock}, shown in your time zone.`
    : "Nothing in this process keeps the schedule — this dashboard is watching only — so when ticks fall is not known here. The queue and the layovers are still shown; the dashboard of the `layover serve` running this factory has the times.";

  const stop = $("#upcoming-stop");
  stop.hidden = !upcoming.ground_stop;
  stop.textContent =
    "A Ground Stop is engaged, so nothing below starts until it is released. A tick that comes due meanwhile fires once, when it is.";
}

// ── Waiting for a slot ──────────────────────────────────────────────────────

function showQueue(queue) {
  const body = $("#queue-table tbody");
  const all = queue.pending;
  const mine = scope() ? all.filter((flight) => flight.pipeline === scope()) : all;
  body.replaceChildren();

  mine.forEach((flight) => {
    const row = el("tr");
    const prompt = el("td", "prompt", flight.body.split("\n")[0]);
    prompt.title = flight.body;
    const cancel = el("button", "link danger", "Cancel");
    cancel.type = "button";
    cancel.title = "Take it off the queue. It has not started, and will not.";
    cancel.addEventListener("click", () => cancelQueued(flight, cancel));
    const actions = el("td", "actions");
    actions.append(cancel);
    const chain = el("td");
    chain.append(chainLink(flight.itinerary_id));
    row.append(
      el("td", "num", `${all.indexOf(flight) + 1}`),
      el("td", "when", when(flight.queued_at)),
      el("td", "", flight.to),
      el("td", "", flight.pipeline ?? "—"),
      chain,
      prompt,
      actions,
    );
    if (!flight.pipeline) {
      row.cells[3].title = "Sent by an agent, in a chain that is already under way";
    }
    body.append(row);
  });

  const running = `${queue.alive_runs} of ${queue.max_concurrent_runs} run(s) alive.`;
  const starter = queue.dispatched_by
    ? `Queued work starts first in, first out, as slots free, by ${queue.dispatched_by}.`
    : "Nothing in this process starts queued work — it is watching only — so it waits for a Tower (`layover serve`) or `layover run`.";
  const hidden = all.length - mine.length;
  $("#queue-note").textContent =
    `${running} ${starter}` +
    (hidden > 0 ? ` ${hidden} more queued for other workflows, or for chains already under way.` : "");

  $("#queue-table").hidden = mine.length === 0;
  const empty = $("#queue-empty");
  empty.hidden = mine.length > 0;
  empty.textContent = "Nothing is waiting for a slot.";
}

async function cancelQueued(flight, button) {
  if (!confirm(`Take the flight to ${flight.to} off the queue? It has not started, and will not.`)) return;
  button.disabled = true;
  try {
    await send(`/flights/${encodeURIComponent(flight.flight_id)}`, "DELETE");
  } catch (error) {
    // Most often it started a moment ago, which is not something a cancel can undo.
    alert(`Not cancelled: ${error.message}`);
  }
  loadUpcoming();
  loadQueued();
}

// ── Scheduled ───────────────────────────────────────────────────────────────

function whatItDoes(fire) {
  if (fire.resumes) {
    return fire.collects > 0
      ? `picks up ${fire.collects} layover${fire.collects === 1 ? "" : "s"}`
      : "looks for layovers that are due";
  }
  const entry = pipelinesByName.get(fire.pipeline)?.entry;
  return entry ? `starts ${entry}` : "starts the workflow";
}

function fireNote(fire, upcoming) {
  if (fire.overdue) {
    return upcoming.ground_stop
      ? ["warn", "held by the Ground Stop; fires once it is released"]
      : ["warn", "due; fires as soon as the Tower can"];
  }
  if (fire.may_skip) {
    return ["warn", "skipped unless its previous run finishes first"];
  }
  return ["", ""];
}

/// Ticks in order, with each run of consecutive ticks of one workflow that have nothing to say about
/// them — not held, not at risk of a skip, collecting nothing — gathered together. A one-minute
/// schedule would otherwise fill the table a row a minute and push every other schedule off it.
function foldTicks(fires) {
  const runs = [];
  for (const fire of fires) {
    const plain = !fire.overdue && !fire.may_skip && fire.collects === 0;
    const last = runs[runs.length - 1];
    if (
      last?.plain &&
      plain &&
      last.fires[0].pipeline === fire.pipeline &&
      dayHeading(last.fires[0].at) === dayHeading(fire.at)
    ) {
      last.fires.push(fire);
    } else {
      runs.push({ plain, fires: [fire] });
    }
  }
  // Two in a row read more easily as two rows than as a range.
  return runs.flatMap((run) => (run.fires.length >= 3 ? [run.fires] : run.fires.map((fire) => [fire])));
}

function showFires(upcoming, now, hours) {
  const body = $("#fires-table tbody");
  const workflows = upcoming.workflows.filter((w) => !scope() || w.pipeline === scope());
  const fires = upcoming.fires.filter((fire) => !scope() || fire.pipeline === scope());
  body.replaceChildren();

  let day = null;
  for (const group of foldTicks(fires)) {
    const [fire, last] = [group[0], group[group.length - 1]];
    const heading = dayHeading(fire.at);
    if (heading !== day) {
      day = heading;
      const row = el("tr", "day");
      const cell = el("th", "", heading);
      cell.colSpan = 5;
      row.append(cell);
      body.append(row);
    }
    const [noteClass, note] = fireNote(fire, upcoming);
    const row = el("tr");
    const at = el(
      "td",
      "when",
      group.length > 1 ? `${timeOfDay(fire.at)} – ${timeOfDay(last.at)}` : timeOfDay(fire.at),
    );
    at.title = new Date(fire.at).toLocaleString();
    const what = group.length > 1 ? `${whatItDoes(fire)} · ${group.length} ticks` : whatItDoes(fire);
    row.append(
      at,
      el("td", "when", fromNow(fire.at, now)),
      el("td", "", fire.pipeline),
      el("td", "", what),
      el("td", noteClass, note),
    );
    body.append(row);
  }

  $("#fires-table").hidden = fires.length === 0;
  const empty = $("#fires-empty");
  empty.hidden = fires.length > 0;
  if (!upcoming.clock) {
    empty.textContent = "No clock here, so no times.";
  } else if (workflows.length === 0) {
    empty.textContent = scope()
      ? `${scope()} has no schedule: it runs when somebody triggers it.`
      : "No workflow has a schedule: they all run when somebody triggers them.";
  } else {
    empty.textContent = `Nothing is due in the next ${hours} hours.`;
  }

  // A one-minute schedule is listed in part, so the hourly one is not buried under it.
  const more = workflows
    .map((w) => [w, fires.filter((fire) => fire.pipeline === w.pipeline).length])
    .filter(([w, listed]) => w.fires_in_window > listed)
    .map(([w, listed]) => `${w.pipeline} fires ${w.fires_in_window.toLocaleString()} times in this window; the first ${listed} are listed.`);
  const note = $("#fires-more");
  note.hidden = more.length === 0;
  note.textContent = more.join(" ");
}

// ── Layovers ────────────────────────────────────────────────────────────────

function pickedUp(layover, upcoming, now) {
  if (layover.collected_at) {
    return ["", `${clockTime(layover.collected_at)} (${fromNow(layover.collected_at, now)}), by ${layover.collected_by}`];
  }
  if (!upcoming.workflows.some((w) => w.resumes)) {
    return ["bad", "never: no scheduled workflow sets resumes = true"];
  }
  return ["", "at the first tick of a resuming workflow after it is due"];
}

function showLayovers(upcoming, now) {
  const body = $("#layovers-table tbody");
  const mine = upcoming.layovers.filter(
    (layover) => !scope() || layover.pipeline === scope() || layover.collected_by === scope(),
  );
  body.replaceChildren();

  for (const layover of mine) {
    const row = el("tr");
    const due = new Date(layover.due_at) <= now ? "due" : `${clockTime(layover.due_at)} (${fromNow(layover.due_at, now)})`;
    const [pickClass, pick] = pickedUp(layover, upcoming, now);
    const by = el("td");
    by.append(el("span", "", `${when(layover.booked_at)} · `), chainLink(layover.booked_by));
    if (layover.pipeline) by.title = `In ${layover.pipeline}`;
    row.append(
      el("td", "", layover.waiting_for),
      el("td", "", layover.agent),
      by,
      el("td", "when", due),
      el("td", pickClass, pick),
    );
    body.append(row);
  }

  $("#layovers-table").hidden = mine.length === 0;
  const empty = $("#layovers-empty");
  empty.hidden = mine.length > 0;
  empty.textContent = "No work is set down to be picked up later.";
}

// ── Skipped ─────────────────────────────────────────────────────────────────

function showSkips(upcoming) {
  const body = $("#skips-table tbody");
  const mine = upcoming.skips.filter((skip) => !scope() || skip.pipeline === scope());
  body.replaceChildren();

  for (const skip of mine) {
    const row = el("tr");
    const at = el("td", "when", when(skip.at));
    at.title = new Date(skip.at).toLocaleString();
    row.append(
      at,
      el("td", "", skip.pipeline),
      el("td", "", skip.reason === "still_working" ? "its previous run had not finished" : skip.reason),
    );
    body.append(row);
  }

  const often = upcoming.workflows
    .filter((w) => (!scope() || w.pipeline === scope()) && w.skipped_7d > 0)
    .map((w) => `${w.pipeline}: ${w.skipped_7d}`);
  const note = $("#skips-note");
  note.hidden = often.length === 0;
  note.textContent = `Skipped in the last 7 days — ${often.join(", ")}. Work that keeps outlasting its interval wants a longer interval, or overlap = "allow" if two at once is safe.`;

  $("#skips-table").hidden = mine.length === 0;
  const empty = $("#skips-empty");
  empty.hidden = mine.length > 0;
  empty.textContent = "No tick was skipped in the last 7 days.";
}

function startUpcoming() {
  $("#upcoming-hours").addEventListener("change", loadUpcoming);
  $("#upcoming-refresh").addEventListener("click", loadUpcoming);
}
