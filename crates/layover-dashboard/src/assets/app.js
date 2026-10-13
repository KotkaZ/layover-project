// The dashboard's behaviour.
//
// Plain ES modules-free script, no framework and no build step. It fetches JSON from the same
// API the specification describes and puts it on the page; the route map arrives already drawn,
// because laying out a graph is done in Rust where it can be unit-tested.
//
// Everything is re-fetched on demand rather than held in a client-side store. The data is small,
// the server is on localhost, and a cache is a second source of truth that can disagree with the
// first — which on a monitoring page is the one failure that matters.

const WINDOWS = [
  ["today", "Today"],
  ["last_24h", "Last 24 hours"],
  ["last_7d", "Last 7 days"],
  ["last_30d", "Last 30 days"],
  ["month_to_date", "Month to date"],
  ["last_90d", "Last 90 days"],
  ["all_time", "All time"],
];

// The windows worth a headline figure. Not all of them: seven numbers is a wall, and these four
// are the ones that answer "is today unusual" and "where is the month going".
const HEADLINES = ["today", "last_7d", "last_30d", "month_to_date"];

async function get(path) {
  const response = await fetch(path, { headers: { accept: "application/json" } });
  if (!response.ok) throw new Error(`${path} returned ${response.status}`);
  return response.json();
}

const $ = (selector) => document.querySelector(selector);
const el = (tag, className, text) => {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
};

const money = (usd) => `$${usd.toFixed(2)}`;

function duration(seconds) {
  if (seconds === null || seconds === undefined) return "—";
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}

function when(iso) {
  if (!iso) return "—";
  const at = new Date(iso);
  const ago = (Date.now() - at.getTime()) / 1000;
  if (ago < 90) return "just now";
  if (ago < 3600) return `${Math.floor(ago / 60)} min ago`;
  if (ago < 172800) return `${Math.floor(ago / 3600)} h ago`;
  if (ago < 30 * 86400) return `${Math.floor(ago / 86400)} d ago`;
  return at.toLocaleDateString();
}

/// A table cell saying how long ago, with the exact moment on hover. Every list reads times the
/// same way, so a help request from yesterday does not look like a different kind of thing from a
/// run an hour ago.
function whenCell(iso, prefix = "") {
  const cell = el("td", "when", `${prefix}${when(iso)}`);
  if (iso) cell.title = new Date(iso).toLocaleString();
  return cell;
}

// How much of a figure is actually measured. Shown next to every total rather than tucked away,
// because a cost that was inferred from a rate card is a guess, and a rate card that has drifted
// from the provider's prices produces a guess that looks exactly like a measurement.
function provenance(summary) {
  if (summary.runs === 0) return ["", ""];
  if (summary.unreported_runs > 0) {
    return ["hole", `${summary.unreported_runs} of ${summary.runs} runs reported nothing`];
  }
  if (summary.estimated_runs > 0) {
    return ["estimate", `${summary.estimated_runs} of ${summary.runs} runs priced from a rate card`];
  }
  // Measured, so not a warning — but named, because the dollars are Layover's arithmetic on the
  // credits Copilot reported, and a price per credit that turned out wrong has to be findable.
  if (summary.credit_runs > 0) {
    return ["", `${summary.credit_runs} of ${summary.runs} runs priced from Copilot credits`];
  }
  return ["", `${summary.runs} runs, all measured`];
}

// A spend figure as it can honestly be stated. Runs that reported nothing count as nothing in the
// sum, so a total built partly from them is a floor and carries a `+`, and one built only from
// them is not a figure at all: "$0.00" there reads as a free factory, and is how an operator whose
// runner prints no cost comes to believe it costs nothing.
function spend(summary) {
  if (summary.runs > 0 && summary.unreported_runs === summary.runs) return "not reported";
  return summary.unreported_runs > 0 ? `${money(summary.usd)}+` : money(summary.usd);
}

function showView(name) {
  document.querySelectorAll(".view").forEach((view) => {
    view.hidden = view.id !== name;
  });
  // A chain is opened from the Chains tab and reads as part of it.
  const tabbed = name === "chain" ? "chains" : name;
  document.querySelectorAll(".tab").forEach((tab) => {
    tab.setAttribute("aria-current", String(tab.dataset.view === tabbed));
  });
  if (name === "map") loadMap();
  if (name === "chains") loadChains();
  if (name === "runs") loadRuns();
  if (name === "cost") loadCost();
  if (name === "journal") loadJournal();
  watchSessions(name === "sessions");
  watchChain(name === "chain");
  watchUpcoming(name === "upcoming");
  routeTo(name);
}

async function loadHealth() {
  const pill = $("#health");
  const button = $("#ground-stop");
  try {
    const health = await get("/health");
    pill.textContent = `v${health.version}${health.ground_stop ? " · ground stop" : ""}`;
    pill.classList.toggle("down", health.ground_stop);

    // The button says what pressing it will do, not what the state is. "Ground Stop" next to a
    // factory that is already stopped reads as though pressing it would stop it again.
    button.classList.toggle("engaged", health.ground_stop);
    button.textContent = health.ground_stop ? "Release" : "Ground Stop";
    button.title = health.ground_stop
      ? "Everything is halted. Press to let work start again."
      : "Halt everything. Running agents are ended; parked work is kept.";
  } catch {
    pill.textContent = "Tower unreachable";
    pill.classList.add("down");
  }
}

// One diagram per workflow. Drawn together they read as a single very confused process, and a
// selector still only lets you see one at a time without being able to compare them.
function rail(kind, label, value) {
  const node = el("span", `rail ${kind}`);
  node.append(document.createTextNode(`${label} `), el("b", "", value));
  return node;
}

function describeTrigger(trigger) {
  if (trigger.kind === "manual") return "manual";
  if (trigger.cron) return `cron ${trigger.cron}`;
  if (trigger.every_seconds) {
    const s = trigger.every_seconds;
    return s % 3600 === 0 ? `every ${s / 3600}h` : `every ${Math.round(s / 60)}m`;
  }
  return "scheduled";
}

/// A workflow's activity in the last seven days, so the diagram is not the only thing on the page.
///
/// A route map shows what *may* happen. This shows what did — and the two questions are asked at
/// the same moment, by someone who has just opened the page wondering whether anything is wrong.
/// `skipped` is null for a workflow with no schedule, which has no ticks to skip.
function summaryStrip(name, stats, skipped) {
  const strip = el("div", "summary");
  const cell = (label, value, className) => {
    const box = el("div", `stat ${className || ""}`);
    box.append(el("span", "figure", value), el("span", "label", label));
    return box;
  };

  strip.append(
    cell("runs, 7d", `${stats.runs}`),
    cell("spend, 7d", spend(stats), stats.unreported_runs > 0 ? "warn" : ""),
    cell("failed", `${stats.failed}`, stats.failed > 0 ? "bad" : ""),
    cell("open help", `${stats.help}`, stats.help > 0 ? "warn" : ""),
  );
  // A schedule that skips every tick has a quiet strip: few runs, nothing failed. This is the
  // number that says why.
  if (skipped !== null) {
    strip.append(cell("skipped ticks, 7d", `${skipped}`, skipped > 0 ? "warn" : ""));
  }
  strip.dataset.workflow = name;
  return strip;
}

/// When a scheduled workflow next fires, as a rail beside its trigger: "next 14:00 · in 23 min".
///
/// Only when the Tower in this process keeps the clock. A watching dashboard does not know, and
/// says nothing rather than a time worked out from a schedule counted from somebody else's start.
function nextRail(scheduled, now) {
  if (!scheduled?.next_at) return null;
  const node = rail("next", "next", `${clockTime(scheduled.next_at)} · ${fromNow(scheduled.next_at, now)}`);
  node.title = new Date(scheduled.next_at).toLocaleString();
  if (scheduled.working && !scheduled.overlaps) {
    node.classList.add("risk");
    node.title += " — its previous run is still going, so this tick is skipped unless that finishes first";
  }
  return node;
}

/// Counts recent activity per workflow in three requests, not three per workflow.
async function activityByWorkflow() {
  const stats = new Map();
  const bump = (name, field, by = 1) => {
    if (!name) return;
    const row = stats.get(name) ?? { runs: 0, usd: 0, unreported_runs: 0, failed: 0, help: 0 };
    row[field] += by;
    stats.set(name, row);
  };

  const [costs, runs, help] = await Promise.all([
    get("/costs?window=last_7d").catch(() => null),
    get("/runs?window=last_7d&status=failed&limit=500").catch(() => null),
    get("/help?open=true&window=last_90d").catch(() => null),
  ]);

  for (const bucket of costs?.by_pipeline ?? []) {
    bump(bucket.name, "runs", bucket.summary.runs);
    bump(bucket.name, "usd", bucket.summary.usd);
    bump(bucket.name, "unreported_runs", bucket.summary.unreported_runs);
  }
  for (const run of runs?.runs ?? []) bump(run.pipeline, "failed");
  for (const request of help?.requests ?? []) bump(request.pipeline, "help");

  return stats;
}

/// Chains still going, or waiting for somebody to answer them.
async function chainsGoing() {
  try {
    const { itineraries } = await get("/itineraries?window=last_7d");
    return itineraries.filter((chain) => ["working", "awaiting_human"].includes(chain.state));
  } catch {
    return [];
  }
}

async function loadMap() {
  const host = $("#workflows");

  try {
    const { pipelines } = await get("/pipelines");
    host.replaceChildren();

    if (pipelines.length === 0) {
      host.append(el("p", "empty", "No pipelines are declared, so nothing can be triggered."));
      return;
    }

    const showing = scope() ? pipelines.filter((p) => p.name === scope()) : pipelines;
    const [activity, going, upcoming] = await Promise.all([
      activityByWorkflow(),
      chainsGoing(),
      get("/upcoming").catch(() => null),
    ]);
    const scheduled = new Map((upcoming?.workflows ?? []).map((w) => [w.pipeline, w]));
    const now = upcoming ? new Date(upcoming.now) : new Date();

    for (const pipeline of showing) {
      const section = el("section", "workflow");
      const header = el("header");
      header.append(el("h2", "", pipeline.name), workflowSwitch(pipeline));
      if (pipeline.description) header.append(el("span", "what", pipeline.description));
      section.classList.toggle("off", pipeline.switched_off);

      const rails = el("div", "rails");
      rails.append(rail("trigger", "", describeTrigger(pipeline.trigger)));
      // Off, a schedule's next tick does not fire, so it is not offered as when it will.
      const next = pipeline.switched_off ? offRail(pipeline) : nextRail(scheduled.get(pipeline.name), now);
      if (next) rails.append(next);
      rails.append(
        // The two rails that bound a chain, and they bound different things: Hops is depth,
        // Fuel is breadth. Showing one without the other invites the assumption that Hops caps
        // spending, which it does not.
        rail("hops", "hops", `${pipeline.max_hops}`),
        rail("fuel", "fuel", money(pipeline.fuel_usd)),
        rail("", "workspace", pipeline.workspace),
      );
      if (pipeline.resumes) rails.append(rail("", "", "resumes layovers"));
      header.append(rails);
      section.append(header);
      section.append(
        summaryStrip(
          pipeline.name,
          activity.get(pipeline.name) ?? { runs: 0, usd: 0, unreported_runs: 0, failed: 0, help: 0 },
          scheduled.get(pipeline.name)?.skipped_7d ?? null,
        ),
      );
      // One map per workflow, however many times it was triggered: these say which run is where.
      const mine = going.filter((chain) => chain.pipeline === pipeline.name);
      if (mine.length) section.append(inFlight(mine));

      const canvas = el("div", "canvas");
      canvas.append(el("p", "empty", "drawing\u2026"));
      section.append(canvas);
      host.append(section);

      get(`/graph?pipeline=${encodeURIComponent(pipeline.name)}`)
        .then((map) => {
          canvas.innerHTML = map.mermaid;
          traceable(canvas, {
            describe: (name) => agentsByName.get(name),
            showRuns: (name) => {
              $("#runs-agent").value = name;
              showView("runs");
            },
          });
          if (map.config_path) $("#config-path").textContent = map.config_path;
        })
        .catch((error) => {
          canvas.replaceChildren(el("p", "empty", `Could not draw it: ${error.message}`));
        });
    }
  } catch (error) {
    host.replaceChildren(el("p", "empty", `Could not read the pipelines: ${error.message}`));
  }
}

// How a chain's state reads on the page. `awaiting_human` is the one that looked finished.
const STATES = { awaiting_human: "waiting for you" };

/// A workflow, and beneath it the name the chain was given when it was triggered, if it was.
function workflowCell(pipeline, name) {
  const cell = el("td");
  if (name) cell.append(el("b", "", name), document.createElement("br"));
  cell.append(el("span", name ? "muted" : "", pipeline ?? "—"));
  return cell;
}

// The kill switch. Confirmed on the way in but not on the way out: stopping should be easy and
// starting again should be deliberate, because the cost of a Ground Stop nobody meant is a pause,
// and the cost of releasing one somebody did mean is whatever they engaged it to prevent.
async function toggleGroundStop() {
  const button = $("#ground-stop");
  const engaged = button.classList.contains("engaged");

  if (engaged && !confirm("Release the Ground Stop? Work will start again.")) return;

  button.disabled = true;
  try {
    const response = await fetch("/ground-stop", {
      method: engaged ? "DELETE" : "POST",
      headers: { accept: "application/json" },
    });
    if (!response.ok) throw new Error(`the Tower answered ${response.status}`);
    await response.json();
  } catch (error) {
    // Said out loud rather than swallowed. A kill switch that fails quietly is worse than one
    // that is not there.
    alert(`Could not change the Ground Stop: ${error.message}`);
  } finally {
    button.disabled = false;
    loadHealth();
  }
}

function costCard(window, label, report, selected) {
  const card = el("div", `card${selected ? " selected" : ""}`);
  const button = el("button");
  button.append(el("span", "k", label), el("span", "v", spend(report.total)));

  const [tone, note] = provenance(report.total);
  if (note) button.append(el("span", `n ${tone}`, note));

  button.addEventListener("click", () => loadCost(window));
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
}

async function loadCost(selected = "last_30d") {
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

    costRows($("#cost-agents"), detail.by_agent);
    costRows($("#cost-models"), detail.by_model);
    costRows($("#cost-pipelines"), detail.by_pipeline);

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
    caveat.textContent = notes.join(" ");
    caveat.hidden = notes.length === 0;
  } catch (error) {
    cards.replaceChildren(el("p", "empty", `Could not read costs: ${error.message}`));
    caveat.hidden = true;
    note.hidden = true;
  }
}

// ── Triggering ───────────────────────────────────────────────────────────────
//
// The dashboard is otherwise read-only. This one control writes, and what it writes is a *queued*
// flight. The Tower behind this page starts it within seconds; the window says what was queued
// rather than pretending it has already run.

let pipelinesByName = new Map();
// Agents as `GET /agents` lists them, for the panel a pinned agent opens on the route map.
let agentsByName = new Map();

/// Sends a JSON body and reads back whatever the Tower says, preferring its own wording for a
/// failure over a bare status code.
async function send(path, method, payload) {
  const response = await fetch(path, {
    method,
    headers: { "content-type": "application/json", accept: "application/json" },
    body: JSON.stringify(payload),
  });
  const value = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(value.detail || value.title || `${response.status}`);
  return value;
}

// Runs alive against the factory's limit, and what is waiting for a slot. Shown whenever either is
// non-zero, because "four of four alive, six queued" is the difference between a busy factory and
// a stuck one.
async function loadQueued() {
  const note = $("#queued");
  try {
    const { pending, dispatched_by: by, alive_runs: alive, max_concurrent_runs: limit } =
      await get("/flights");
    note.hidden = pending.length === 0 && alive === 0;
    if (note.hidden) return;

    setLiveBadge(alive);
    // Amber only when queued work has nothing to start it; a busy factory is news, not trouble.
    note.classList.toggle("caveat", pending.length > 0 && !by);
    const running = `${alive} of ${limit} run(s) alive`;
    if (pending.length === 0) {
      note.textContent = `${running}.`;
    } else if (by) {
      note.textContent = `${running} · ${pending.length} flight(s) queued, started by ${by} as slots free. `;
    } else {
      note.textContent = `${running} · ${pending.length} flight(s) queued. Nothing in this process starts them — it is watching only — so they wait for a Tower (\`layover serve\`) or \`layover run\`. `;
    }
    if (pending.length > 0) {
      const see = el("button", "link", "See the queue");
      see.type = "button";
      see.addEventListener("click", () => showView("upcoming"));
      note.append(see);
    }
  } catch {
    note.hidden = true;
  }
}

async function openReport(runId, run) {
  const transcript = $("#report-transcript");
  transcript.hidden = !run;
  transcript.onclick = () => {
    $("#report").close();
    openSession(run);
  };
  const onward = $("#report-continue");
  onward.hidden = !run?.pipeline;
  onward.onclick = () => {
    $("#report").close();
    continueChain(run.pipeline, run.flags ?? null, run.itinerary_id, run.chain_name);
  };
  try {
    const report = await get(`/runs/${encodeURIComponent(runId)}/report`);
    $("#report-headline").textContent = report.headline;
    $("#report-meta").textContent =
      `${report.agent} · ${when(report.at)}${report.trimmed ? " · trimmed to fit" : ""}`;
    $("#report-body").textContent = report.body || "(no body)";

    const artifacts = $("#report-artifacts");
    artifacts.replaceChildren();
    for (const name of report.artifacts ?? []) artifacts.append(el("code", "", name));

    $("#report").showModal();
  } catch (error) {
    $("#report-headline").textContent = "No report";
    $("#report-meta").textContent = error.message;
    $("#report-body").textContent =
      "An agent writes a report when it finishes. This run either has not, or predates reports.";
    $("#report-artifacts").replaceChildren();
    $("#report").showModal();
  }
}

// Marks every open request from one run as dealt with.
async function resolveHelp(runId, button) {
  button.disabled = true;
  try {
    await send("/help/resolve", "POST", { run_id: runId, window: "last_90d" });
    loadJournal();
    loadHelpBadge();
  } catch (error) {
    button.disabled = false;
    alert(`Could not resolve it: ${error.message}`);
  }
}

// Settles a learning either way. Rejecting is confirmed because it takes something out of every
// future run; keeping is not, because it only stops something already in use from lapsing.
async function judge(id, state, button) {
  if (state === "rejected" && !confirm("Drop this learning? Future runs will stop seeing it.")) {
    return;
  }

  button.disabled = true;
  try {
    await send(`/learnings/${encodeURIComponent(id)}`, "PATCH", { state });
    loadJournal();
  } catch (error) {
    button.disabled = false;
    alert(`Could not change it: ${error.message}`);
  }
}

// The count of open help requests sits on the tab, because a blocked factory is the one thing
// worth seeing without navigating to it.
async function loadHelpBadge() {
  const badge = $("#help-badge");
  try {
    const { open } = await get(
      `/help?${scoped(new URLSearchParams({ open: "true", window: "last_90d" }))}`,
    );
    badge.textContent = `${open}`;
    badge.hidden = open === 0;
  } catch {
    badge.hidden = true;
  }
}

// A factory holds several pipelines and they are separate workflows. One selector in the header
// scopes the whole page, because a workflow is the unit people actually think in — "is the build
// healthy" is a question about one of them, and answering it from a page that totals all three
// means doing the separation by eye.
//
// Learnings are the deliberate exception: a learning belongs to an agent, and an agent can appear
// in several workflows. Narrowing them by workflow would invent an attribution the model does not
// have.
const scope = () => $("#scope").value;

/// Appends `pipeline=` when a workflow is selected, and nothing when it is not.
function scoped(params) {
  if (scope()) params.set("pipeline", scope());
  return params;
}

async function loadPipelines() {
  try {
    const { pipelines } = await get("/pipelines");
    pipelinesByName = new Map(pipelines.map((pipeline) => [pipeline.name, pipeline]));

    const select = $("#scope");
    const previous = select.value;
    const all = el("option", "", "All workflows");
    all.value = "";
    select.replaceChildren(
      all,
      ...pipelines.map((pipeline) => {
        const option = el("option", "", pipeline.name);
        option.value = pipeline.name;
        return option;
      }),
    );
    if (previous) select.value = previous;
    $("#scope-label").hidden = pipelines.length < 2;
  } catch {
    // Without the list the selector still reads as "everything"; nothing needs saying.
  }
}

async function loadAgentNames() {
  try {
    const { agents } = await get("/agents");
    agentsByName = new Map(agents.map((agent) => [agent.name, agent]));
    $("#agent-names").replaceChildren(
      ...agents.map((agent) => {
        const option = el("option");
        option.value = agent.name;
        return option;
      }),
    );
  } catch {
    // A missing autocomplete list is not worth telling anybody about; the field still works.
  }
}

function start() {
  $("#runs-window").replaceChildren(
    ...WINDOWS.map(([slug, label]) => {
      const option = el("option", "", label);
      option.value = slug;
      option.selected = slug === "last_7d";
      return option;
    }),
  );

  $("#chains-window").replaceChildren(
    ...WINDOWS.map(([slug, label]) => {
      const option = el("option", "", label);
      option.value = slug;
      option.selected = slug === "last_7d";
      return option;
    }),
  );

  document.querySelectorAll(".tab").forEach((tab) => {
    tab.addEventListener("click", () => showView(tab.dataset.view));
  });
  $("#refresh").addEventListener("click", loadMap);
  startTrigger();
  $("#ground-stop").addEventListener("click", toggleGroundStop);
  startChainsList();
  startRuns();
  startJournal();
  startSessions();
  startReplies();
  startChains();
  startUpcoming();

  // One selector, so every view has to be told. Redrawing only the visible one would leave the
  // others showing another workflow's numbers under this workflow's name the moment you switch.
  $("#scope").addEventListener("change", () => {
    $("#learn-scope").hidden = !scope();
    routeChanged();
    loadMap();
    loadChains();
    loadRuns();
    loadCost();
    loadJournal();
    loadHelpBadge();
    if (!$("#sessions").hidden) loadSessions();
    if (!$("#upcoming").hidden) loadUpcoming();
  });

  loadHealth();
  loadChains();
  loadHelpBadge();
  loadQueued();
  // Runs start and finish on their own, so the count is kept current rather than drawn once.
  setInterval(loadQueued, 5000);
  // The pipeline list has to exist before the first draw, or the selector is empty on load —
  // and before the address is read, since it may name a workflow.
  loadPipelines().then(() => {
    loadAgentNames();
    startRouter();
    applyRoute();
  });
}

start();
