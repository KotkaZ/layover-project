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
  if (name === "runs") loadRuns();
  if (name === "cost") loadCost();
  if (name === "journal") loadJournal();
}

async function loadHealth() {
  const pill = $("#health");
  try {
    const health = await get("/health");
    pill.textContent = `v${health.version}${health.ground_stop ? " · ground stop" : ""}`;
    pill.classList.toggle("down", health.ground_stop);
  } catch {
    pill.textContent = "Tower unreachable";
    pill.classList.add("down");
  }
}

async function loadMap() {
  const canvas = $("#routemap");
  const wanted = $("#graph-pipeline").value;
  try {
    const map = await get(wanted ? `/graph?pipeline=${encodeURIComponent(wanted)}` : "/graph");
    canvas.innerHTML = map.mermaid;
    if (map.config_path) $("#config-path").textContent = map.config_path;
  } catch (error) {
    canvas.replaceChildren(el("p", "empty", `Could not draw the route map: ${error.message}`));
  }
}

async function loadRuns() {
  const body = $("#runs-table tbody");
  const empty = $("#runs-empty");
  const params = new URLSearchParams({ window: $("#runs-window").value, limit: "200" });
  if ($("#runs-status").value) params.set("status", $("#runs-status").value);
  if ($("#runs-agent").value.trim()) params.set("agent", $("#runs-agent").value.trim());
  if ($("#runs-pipeline").value) params.set("pipeline", $("#runs-pipeline").value);

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

async function loadCost(selected = "last_30d") {
  const cards = $("#cost-cards");
  const caveat = $("#cost-caveat");

  try {
    const reports = await Promise.all(
      HEADLINES.map(async (window) => [window, await get(`/costs?window=${window}`)]),
    );

    cards.replaceChildren();
    for (const [window, report] of reports) {
      const label = WINDOWS.find(([slug]) => slug === window)[1];
      cards.append(costCard(window, label, report, window === selected));
    }

    const detail = reports.find(([window]) => window === selected)?.[1]
      ?? (await get(`/costs?window=${selected}`));

    costRows($("#cost-agents"), detail.by_agent);
    costRows($("#cost-models"), detail.by_model);
    costRows($("#cost-pipelines"), detail.by_pipeline);

    // A calendar window's start depends on where the Tower is standing, so the zone is part of
    // the number rather than a footnote. A window that outruns retention is a lower bound.
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
  }
}

async function loadJournal() {
  const helpBody = document.querySelector("#help-table tbody");
  const helpEmpty = $("#help-empty");

  try {
    const { requests } = await get("/help?open=true&window=last_90d");
    helpBody.replaceChildren();

    for (const request of requests) {
      const row = el("tr");
      const kind = el("td");
      kind.append(el("span", `kind ${request.blocker}`, request.blocker));
      row.append(el("td", "", when(request.at)), el("td", "", request.agent), kind);
      const what = el("td", "wide", request.summary);
      what.title = request.detail;
      row.append(what, el("td", "", request.fatal ? "stopped the run" : "limited it"));
      helpBody.append(row);
    }

    $("#help-table").hidden = requests.length === 0;
    helpEmpty.hidden = requests.length > 0;
    helpEmpty.textContent = "Nothing is stuck.";
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
    const { open } = await get("/help?open=true&window=last_90d");
    badge.textContent = `${open}`;
    badge.hidden = open === 0;
  } catch {
    badge.hidden = true;
  }
}

// A factory holds several pipelines and they are separate workflows. Every view can be narrowed
// to one, because drawn or totalled together they read as a single very confused process.
async function loadPipelines() {
  try {
    const { pipelines } = await get("/pipelines");
    for (const id of ["#graph-pipeline", "#runs-pipeline"]) {
      const select = $(id);
      const any = el("option", "", id === "#graph-pipeline" ? "everything" : "any");
      any.value = "";
      select.replaceChildren(
        any,
        ...pipelines.map((pipeline) => {
          const option = el("option", "", pipeline.name);
          option.value = pipeline.name;
          return option;
        }),
      );
    }
  } catch {
    // Without the list both selectors still work as "everything"; nothing needs saying.
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

  document.querySelectorAll(".tab").forEach((tab) => {
    tab.addEventListener("click", () => showView(tab.dataset.view));
  });
  $("#refresh").addEventListener("click", loadMap);
  $("#graph-pipeline").addEventListener("change", loadMap);
  $("#runs-pipeline").addEventListener("change", loadRuns);
  ["#runs-window", "#runs-status"].forEach((id) => $(id).addEventListener("change", loadRuns));
  $("#runs-agent").addEventListener("input", loadRuns);

  loadHealth();
  loadHelpBadge();
  // The pipeline list has to exist before the first draw, or the selector is empty on load.
  loadPipelines().then(() => {
    loadAgentNames();
    showView("map");
  });
}

start();
