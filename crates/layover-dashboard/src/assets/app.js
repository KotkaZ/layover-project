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
  try {
    const map = await get("/graph");
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
  ["#runs-window", "#runs-status"].forEach((id) => $(id).addEventListener("change", loadRuns));
  $("#runs-agent").addEventListener("input", loadRuns);

  loadHealth();
  loadAgentNames();
  showView("map");
}

start();
