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

// What refreshes itself while it is on screen, and how. Sessions, Upcoming and a chain keep
// their own, faster, clocks.
const LIVE = { map: () => loadMap(true), chains: () => loadChains(), runs: () => loadRuns() };
const LIVE_EVERY_MS = 15000;
let liveTimer = null;

function watchLive(name) {
  clearInterval(liveTimer);
  liveTimer = null;
  const refresh = LIVE[name];
  if (!refresh) return;
  liveTimer = setInterval(() => {
    if (!document.hidden) refresh();
  }, LIVE_EVERY_MS);
}

/// Says when a view last read the Tower, so a page left open is known to be current.
function updated(selector) {
  const stamp = $(selector);
  const now = new Date();
  stamp.textContent = `updated ${now.toLocaleTimeString([], { hour12: false })}`;
  stamp.title = `Read from the Tower at ${now.toLocaleString()}; kept current every ${LIVE_EVERY_MS / 1000} s while this tab is showing.`;
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
  watchLive(name);
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
  onward.hidden = !startable(run?.pipeline);
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
    noteAttention({ help: open, helpScope: scope() });
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
  startMap();
  startTrigger();
  $("#ground-stop").addEventListener("click", toggleGroundStop);
  startChainsList();
  startRuns();
  startJournal();
  startSessions();
  startReplies();
  startChains();
  startUpcoming();
  startAttention();
  // Back to a tab left in the background, it is brought up to date at once rather than at the
  // next tick.
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) LIVE[currentView()]?.();
  });

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
