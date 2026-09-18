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
  if (ago < 86400) return `${Math.floor(ago / 3600)} h ago`;
  return at.toLocaleString();
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
  return ["", `${summary.runs} runs, all measured`];
}

function showView(name) {
  document.querySelectorAll(".view").forEach((view) => {
    view.hidden = view.id !== name;
  });
  document.querySelectorAll(".tab").forEach((tab) => {
    tab.setAttribute("aria-current", String(tab.dataset.view === name));
  });
  if (name === "map") loadMap();
  if (name === "chains") loadChains();
  if (name === "runs") loadRuns();
  if (name === "cost") loadCost();
  if (name === "journal") loadJournal();
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
function summaryStrip(name, stats) {
  const strip = el("div", "summary");
  const cell = (label, value, className) => {
    const box = el("div", `stat ${className || ""}`);
    box.append(el("span", "figure", value), el("span", "label", label));
    return box;
  };

  strip.append(
    cell("runs, 7d", `${stats.runs}`),
    cell("spend, 7d", money(stats.usd)),
    cell("failed", `${stats.failed}`, stats.failed > 0 ? "bad" : ""),
    cell("open help", `${stats.help}`, stats.help > 0 ? "warn" : ""),
  );
  strip.dataset.workflow = name;
  return strip;
}

/// Counts recent activity per workflow in three requests, not three per workflow.
async function activityByWorkflow() {
  const stats = new Map();
  const bump = (name, field, by = 1) => {
    if (!name) return;
    const row = stats.get(name) ?? { runs: 0, usd: 0, failed: 0, help: 0 };
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
  }
  for (const run of runs?.runs ?? []) bump(run.pipeline, "failed");
  for (const request of help?.requests ?? []) bump(request.pipeline, "help");

  return stats;
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
    const activity = await activityByWorkflow();

    for (const pipeline of showing) {
      const section = el("section", "workflow");
      const header = el("header");
      header.append(el("h2", "", pipeline.name));
      if (pipeline.description) header.append(el("span", "what", pipeline.description));

      const rails = el("div", "rails");
      rails.append(
        rail("trigger", "", describeTrigger(pipeline.trigger)),
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
          activity.get(pipeline.name) ?? { runs: 0, usd: 0, failed: 0, help: 0 },
        ),
      );

      const canvas = el("div", "canvas");
      canvas.append(el("p", "empty", "drawing\u2026"));
      section.append(canvas);
      host.append(section);

      get(`/graph?pipeline=${encodeURIComponent(pipeline.name)}`)
        .then((map) => {
          canvas.innerHTML = map.mermaid;
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

async function loadRuns() {
  const body = $("#runs-table tbody");
  const empty = $("#runs-empty");
  const params = new URLSearchParams({ window: $("#runs-window").value, limit: "200" });
  if ($("#runs-status").value) params.set("status", $("#runs-status").value);
  if ($("#runs-agent").value.trim()) params.set("agent", $("#runs-agent").value.trim());
  scoped(params);

  try {
    const { runs } = await get(`/runs?${params}`);
    body.replaceChildren();

    for (const run of runs) {
      const row = el("tr");
      row.append(
        el("td", "", when(run.finished_at ?? run.started_at)),
        el("td", "", run.agent),
        el("td", "", run.pipeline ?? "—"),
        el("td", `outcome ${run.status}`, run.status.replace("_", " ")),
        el("td", "num", duration(run.duration_sec)),
        el("td", "num", run.cost_usd === null ? "not reported" : money(run.cost_usd)),
      );
      const chain = el("td");
      chain.append(el("code", "", run.itinerary_id));
      row.append(chain);
      if (run.detail) row.title = run.detail;
      // Every row opens whatever the agent wrote about itself.
      row.classList.add("readable");
      row.addEventListener("click", () => openReport(run.run_id));
      body.append(row);
    }

    empty.hidden = runs.length > 0;
    empty.textContent = "Nothing ran in this window.";
    $("#runs-table").hidden = runs.length === 0;
  } catch (error) {
    body.replaceChildren();
    $("#runs-table").hidden = true;
    empty.hidden = false;
    empty.textContent = `Could not read history: ${error.message}`;
  }
}

// A chain is what a person actually asked for; a run is one step of it. Shown separately because
// the interesting failure — a chain that stopped with every run reporting success — is invisible
// in a list of runs, which is where somebody would otherwise go looking for it.
async function loadChains() {
  const body = $("#chains-table tbody");
  const count = $("#chains-count");
  const params = new URLSearchParams({ window: $("#chains-window").value });
  if ($("#chains-state").value) params.set("state", $("#chains-state").value);

  try {
    const { itineraries, stalled } = await get(`/itineraries?${params}`);
    body.replaceChildren();

    for (const chain of itineraries) {
      const row = el("tr");
      const cost = chain.measured === false ? `${money(chain.usd)}+` : money(chain.usd);

      row.append(
        el("td", "", when(chain.started_at)),
        el("td", "", chain.pipeline ?? "—"),
        el("td", "", (chain.agents ?? []).join(" → ") || "—"),
        el("td", "num", String(chain.runs)),
        el("td", "num", cost),
        el("td", `outcome ${chain.state}`, chain.state),
      );

      // The reason lives in a tooltip rather than a column: it is a sentence, and a column wide
      // enough for it would squeeze out everything that is scannable.
      if (chain.detail) row.title = chain.detail;
      body.append(row);
    }

    $("#chains-table").hidden = itineraries.length === 0;
    count.textContent = itineraries.length === 0
      ? "Nothing ran in this window."
      : `${itineraries.length} chain(s)${stalled ? `, ${stalled} stalled` : ""}`;

    const badge = $("#stalled-badge");
    badge.textContent = String(stalled);
    badge.hidden = stalled === 0;
  } catch (error) {
    body.replaceChildren();
    $("#chains-table").hidden = true;
    count.textContent = `Could not read chains: ${error.message}`;
  }
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
  button.append(el("span", "k", label), el("span", "v", money(report.total.usd)));

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
      el("td", `num ${tone}`, money(bucket.summary.usd)),
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
// flight: nothing dispatches it yet, because dispatching needs the supervisor. The window says so
// rather than presenting a button that appears to start work and does not -- a control trusted
// once and then relied upon is worse than one that was never offered.

let pipelinesByName = new Map();

async function post(path, payload) {
  const response = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json", accept: "application/json" },
    body: JSON.stringify(payload),
  });
  const value = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(value.detail || value.title || `${response.status}`);
  return value;
}

function showFlags(name) {
  const box = $("#trigger-flags");
  const pipeline = pipelinesByName.get(name);
  box.querySelectorAll(".flag").forEach((node) => node.remove());

  const flags = pipeline?.flags ?? [];
  $("#trigger-noflags").hidden = flags.length > 0;

  for (const flag of flags) {
    const row = el("label", "flag");
    const box2 = document.createElement("input");
    box2.type = "checkbox";
    box2.dataset.flag = flag.name;
    // Start from the declared default, so the window shows what would happen if you changed
    // nothing rather than a set of switches that all read false.
    box2.checked = flag.default;

    const text = el("span");
    text.append(el("span", "n", flag.name));
    if (flag.description) text.append(document.createElement("br"), el("span", "d", flag.description));

    row.append(box2, text);
    box.append(row);
  }
}

async function openTrigger() {
  const select = $("#trigger-pipeline");
  select.replaceChildren(
    ...[...pipelinesByName.values()].map((pipeline) => {
      const option = el("option", "", pipeline.name);
      option.value = pipeline.name;
      return option;
    }),
  );

  $("#trigger-error").hidden = true;
  $("#trigger-body").value = "";
  showFlags(select.value);

  const { dispatched_by: by } = await get("/flights").catch(() => ({ dispatched_by: null }));
  $("#trigger-note").textContent = by
    ? `Queued work is picked up by ${by}.`
    : "This queues the work. Nothing dispatches it yet — the supervisor is not part of this release, so it will sit in the queue until there is something to run it.";

  $("#trigger").showModal();
}

async function submitTrigger(event) {
  const pipeline = $("#trigger-pipeline").value;
  const body = $("#trigger-body").value.trim();

  if (!body) {
    event.preventDefault();
    $("#trigger-error").hidden = false;
    $("#trigger-error").textContent = "Give the entry agent something to start from.";
    return;
  }

  const flags = {};
  for (const box of $("#trigger-flags").querySelectorAll("input[type=checkbox]")) {
    flags[box.dataset.flag] = box.checked;
  }

  event.preventDefault();
  try {
    await post("/flights", { pipeline, body, flags });
    $("#trigger").close();
    loadQueued();
  } catch (error) {
    $("#trigger-error").hidden = false;
    $("#trigger-error").textContent = `Not queued: ${error.message}`;
  }
}

async function loadQueued() {
  const note = $("#queued");
  try {
    const { pending, dispatched_by: by } = await get("/flights");
    note.hidden = pending.length === 0;
    if (pending.length === 0) return;
    note.textContent = by
      ? `${pending.length} flight(s) queued, waiting on ${by}.`
      : `${pending.length} flight(s) queued. Nothing will dispatch them until a supervisor exists.`;
  } catch {
    note.hidden = true;
  }
}

async function openReport(runId) {
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

async function loadJournal() {
  const helpBody = document.querySelector("#help-table tbody");
  const helpEmpty = $("#help-empty");

  try {
    const { requests } = await get(
      `/help?${scoped(new URLSearchParams({ open: "true", window: "last_90d" }))}`,
    );
    helpBody.replaceChildren();

    for (const request of requests) {
      const row = el("tr");
      const kind = el("td");
      kind.append(el("span", `kind ${request.blocker}`, request.blocker));
      row.append(
        el("td", "", when(request.at)),
        el("td", "", request.agent),
        el("td", "", request.pipeline || "\u2014"),
        kind,
      );
      const what = el("td", "wide", request.summary);
      what.title = request.detail;
      row.append(what, el("td", "", request.fatal ? "stopped the run" : "limited it"));
      helpBody.append(row);
    }

    $("#help-table").hidden = requests.length === 0;
    helpEmpty.hidden = requests.length > 0;
    helpEmpty.textContent = scope()
      ? `Nothing is stuck in ${scope()}.`
      : "Nothing is stuck.";
  } catch (error) {
    $("#help-table").hidden = true;
    helpEmpty.hidden = false;
    helpEmpty.textContent = `Could not read help requests: ${error.message}`;
  }

  const learnBody = document.querySelector("#learn-table tbody");
  const learnEmpty = $("#learn-empty");

  try {
    const { learnings } = await get("/learnings");
    learnBody.replaceChildren();

    // Active first, then by how well established. A lapsed learning is still worth seeing: it is
    // what a rediscovery would revive, so it explains why a repeat proposal counted.
    const order = { confirmed: 0, provisional: 1, lapsed: 2, rejected: 3 };
    learnings.sort((a, b) => order[a.state] - order[b.state] || b.proposals - a.proposals);

    for (const learning of learnings) {
      const row = el("tr");
      row.append(el("td", "", learning.agent), el("td", "wide", learning.text));
      row.append(el("td", `standing ${learning.state}`, learning.state));
      row.append(
        el("td", "num", `${learning.proposals}\u00d7`),
        el("td", "num", learning.state === "provisional" ? `${learning.runs_left}` : "\u2014"),
      );
      row.title = `impact: ${learning.impact}`;
      learnBody.append(row);
    }

    $("#learn-table").hidden = learnings.length === 0;
    learnEmpty.hidden = learnings.length > 0;
    learnEmpty.textContent = "Nothing learned yet.";
  } catch (error) {
    $("#learn-table").hidden = true;
    learnEmpty.hidden = false;
    learnEmpty.textContent = `Could not read learnings: ${error.message}`;
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
  $("#trigger-open").addEventListener("click", openTrigger);
  $("#trigger-pipeline").addEventListener("change", (e) => showFlags(e.target.value));
  $("#trigger-send").addEventListener("click", submitTrigger);
  $("#ground-stop").addEventListener("click", toggleGroundStop);
  ["#runs-window", "#runs-status"].forEach((id) => $(id).addEventListener("change", loadRuns));
  ["#chains-window", "#chains-state"].forEach((id) =>
    $(id).addEventListener("change", loadChains),
  );
  $("#runs-agent").addEventListener("input", loadRuns);

  // One selector, so every view has to be told. Redrawing only the visible one would leave the
  // others showing another workflow's numbers under this workflow's name the moment you switch.
  $("#scope").addEventListener("change", () => {
    $("#learn-scope").hidden = !scope();
    loadMap();
    loadChains();
    loadRuns();
    loadCost();
    loadJournal();
    loadHelpBadge();
  });

  loadHealth();
  loadChains();
  loadHelpBadge();
  loadQueued();
  // The pipeline list has to exist before the first draw, or the selector is empty on load.
  loadPipelines().then(() => {
    loadAgentNames();
    showView("map");
  });
}

start();
