// Sessions: every run the factory supervises — the ones going now, live, and every one before them.
//
// One tab for what used to be two. A list of runs and a list of sessions were the same runs, with
// and without filters, and a session was only ever a run's terminal. So the live view sits on top —
// every session running now, side by side, each added as it starts — and the history under it is
// where any other one is picked from, to watch live or replay once it is over.
//
// Uses app.js's helpers (`get`, `el`, `$`, `money`, `duration`, `when`, `whenCell`, `workflowCell`,
// `scoped`, `openReport`, `updated`, `showView`, `WINDOWS`), router.js's `routeChanged` and
// `routeExtras`, terminal.js's `Terminal` and chain.js's `openChain`, all inside functions.

// More live terminals than this would hold enough connections open to starve the rest of the page.
const MAX_TILES = 4;
const SESSIONS_PAGE = 100;
// The live view is kept current this often, and the history under it every third time.
const SESSIONS_EVERY_MS = 5000;

// `picked`: the session somebody chose to watch on its own, or null while the view follows what is
// running. `running`: what was running when last asked, for the workflow in view. `closed`: live
// sessions somebody closed, which the next refresh must not put straight back.
const sessions = {
  terminals: new Map(),
  picked: null,
  running: [],
  closed: new Set(),
  limit: 0,
  timer: null,
  ticks: 0,
};

// ── The live view ──────────────────────────────────────────────────────────────────────────────

function watch(run) {
  if (sessions.terminals.has(run.run_id)) return;
  sessions.terminals.set(run.run_id, new Terminal(run, $("#terminals"), forget));
}

function forget(terminal) {
  terminal.close();
  sessions.terminals.delete(terminal.run.run_id);
  // Closing the one session picked goes back to what is live.
  if (sessions.picked === terminal.run.run_id) {
    sessions.picked = null;
    followLive();
    routeChanged();
  } else {
    sessions.closed.add(terminal.run.run_id);
  }
  drawViewer();
}

function closeAll() {
  for (const terminal of sessions.terminals.values()) terminal.close();
  sessions.terminals.clear();
}

/// Every session running now, side by side: one that starts joins, and one that ends stays until
/// its place is needed, so how it ended can be read.
function followLive() {
  for (const run of sessions.running) {
    if (sessions.terminals.has(run.run_id) || sessions.closed.has(run.run_id)) continue;
    if (sessions.terminals.size >= MAX_TILES) {
      const over = [...sessions.terminals.values()].find((terminal) => terminal.ended);
      if (!over) break;
      over.close();
      sessions.terminals.delete(over.run.run_id);
    }
    watch(run);
  }
}

/// Watches one session on its own: live while it runs, a replay once it is over.
function pick(run) {
  sessions.picked = run.run_id;
  closeAll();
  watch(run);
  drawViewer();
  routeChanged();
}

function backToLive() {
  sessions.picked = null;
  sessions.closed.clear();
  closeAll();
  followLive();
  drawViewer();
  routeChanged();
}

/// Opens the Sessions tab on one session: from a chain, a report or a timeline.
function openSession(run) {
  pick(run);
  showView("sessions");
  window.scrollTo({ top: 0 });
}

/// Opens the Sessions tab on one agent's history: from the route map or a chain's map.
function sessionsOf(agent) {
  $("#sessions-agent").value = agent;
  sessions.limit = 0;
  showView("sessions");
}

/// The heading over the terminals, and which rows of the history are on screen.
function drawViewer() {
  const running = sessions.running.length;
  const picked = sessions.picked && sessions.terminals.get(sessions.picked);
  const heading = $("#watch-heading");
  if (picked) {
    const live = picked.run.status === "running" && !picked.ended;
    heading.replaceChildren(
      el("span", `dot ${live ? "running" : picked.run.status}`),
      el("span", "", `${live ? "Watching" : "Replaying"} ${picked.run.agent}`),
    );
  } else {
    heading.replaceChildren(
      el("span", `dot ${running ? "running" : ""}`),
      el("span", "", running ? `Live · ${running} running` : "Nothing running"),
    );
    if (running > MAX_TILES) {
      heading.append(el("span", "muted", `${MAX_TILES} shown; pick another below to watch it`));
    }
  }
  const back = $("#watch-live");
  back.hidden = !sessions.picked;
  back.textContent = running ? `← Live · ${running} running` : "← Live";

  const shown = sessions.terminals.size;
  $("#terminals").classList.toggle("tiled", shown > 1);
  $("#terminals").classList.toggle("crowded", shown > 2);
  $("#sessions-empty").hidden = shown > 0;
  document.querySelectorAll("#sessions-table tbody tr").forEach((row) => {
    row.classList.toggle("open", sessions.terminals.has(row.dataset.run));
  });
}

function setLiveBadge(count) {
  const badge = $("#live-badge");
  badge.textContent = String(count);
  badge.hidden = count === 0;
}

async function loadLive() {
  try {
    const params = scoped(new URLSearchParams({ status: "running", window: "last_24h", limit: "50" }));
    sessions.running = (await get(`/runs?${params}`)).runs;
    if (!sessions.picked) followLive();
    drawViewer();
  } catch {
    // The history under it says when the Tower cannot be read; one line of that is enough.
  }
}

// ── The history ────────────────────────────────────────────────────────────────────────────────

/// A chain's identifier, short enough to scan: the end of it, which is where ULIDs differ.
function shortChain(id) {
  return id ? `…${id.slice(-6)}` : "—";
}

/// What a run ran on, as its command line said: the model, and its effort where one was set.
function ranOn(run) {
  const cell = el("td", "muted ran");
  if (!run.model) {
    cell.textContent = "—";
    return cell;
  }
  cell.textContent = run.reasoning_effort ? `${run.model} · ${run.reasoning_effort}` : run.model;
  cell.title = [run.model, run.reasoning_effort && `effort ${run.reasoning_effort}`, run.context]
    .filter(Boolean)
    .join(" · ");
  return cell;
}

/// How a run ended — and, for a failure that was not the run's own, whose it was.
function outcomeCell(run) {
  const cell = el("td", `outcome ${run.status}`);
  cell.append(el("span", `dot ${run.status}`), run.status.replace("_", " "));
  if (run.cause === "provider_error") {
    cell.append(el("span", "cause", " · model service"));
    // A rejected call, or a model it did not have when the run started: the detail says which, in
    // the CLI's own words.
    cell.title = `The model service ended the run, not its work${run.request_id ? ` (request ${run.request_id})` : ""}. ${run.detail ?? ""}`.trim();
  }
  return cell;
}

function sessionRow(run) {
  const row = el("tr", "readable");
  row.dataset.run = run.run_id;
  // A run still going has not finished: say when it started, and that it is still going.
  row.append(
    run.status === "running"
      ? whenCell(run.started_at, "started ")
      : whenCell(run.finished_at ?? run.started_at),
    el("td", "agent", run.agent),
    workflowCell(run.pipeline, run.chain_name),
    ranOn(run),
    outcomeCell(run),
    el("td", "num", duration(run.duration_sec)),
    // A run still going has not been billed yet, which is not the same as reporting nothing.
    el(
      "td",
      "num",
      run.status === "running" ? "—" : run.cost_usd === null ? "not reported" : money(run.cost_usd),
    ),
  );

  const chain = el("td");
  const open = el("button", "link", shortChain(run.itinerary_id));
  open.type = "button";
  open.title = `${run.chain_name ? `${run.chain_name} — ` : ""}${run.itinerary_id}: see this chain whole`;
  open.addEventListener("click", (event) => {
    event.stopPropagation();
    openChain(run.itinerary_id);
  });
  chain.append(open);

  const actions = el("td", "actions");
  const report = el("button", "link", "Report");
  report.type = "button";
  report.title = "What the agent wrote about this run";
  report.addEventListener("click", (event) => {
    event.stopPropagation();
    openReport(run.run_id, run);
  });
  actions.append(report);
  row.append(chain, actions);

  row.title = `${run.status === "running" ? "Watch it live" : "Replay it"}${run.detail ? ` — ${run.detail}` : ""}`;
  row.addEventListener("click", () => {
    pick(run);
    // The viewer is the top of the tab; scrolling an element into view would tuck its heading
    // under the page's sticky header.
    window.scrollTo({ top: 0, behavior: "smooth" });
  });
  return row;
}

/// Loads the newest sessions that pass the filters; `more` asks for the next page as well.
async function loadHistory(more = false) {
  const body = $("#sessions-table tbody");
  const empty = $("#sessions-none");
  sessions.limit = more ? sessions.limit + SESSIONS_PAGE : Math.max(sessions.limit, SESSIONS_PAGE);
  const params = new URLSearchParams({
    window: $("#sessions-window").value,
    limit: String(sessions.limit + 1),
  });
  if ($("#sessions-status").value) params.set("status", $("#sessions-status").value);
  if ($("#sessions-agent").value.trim()) params.set("agent", $("#sessions-agent").value.trim());
  scoped(params);

  try {
    const { runs } = await get(`/runs?${params}`);
    // One more than shown is asked for, so whether there is another page is known without a
    // second request.
    const hasMore = runs.length > sessions.limit;
    body.replaceChildren(...runs.slice(0, sessions.limit).map(sessionRow));

    empty.hidden = runs.length > 0;
    empty.textContent = $("#sessions-status").value || $("#sessions-agent").value.trim()
      ? "No session in this window matches."
      : "Nothing ran in this window.";
    $("#sessions-table").hidden = runs.length === 0;
    const button = $("#sessions-more");
    button.hidden = !hasMore;
    button.textContent = `Show ${SESSIONS_PAGE} more`;
    updated("#sessions-updated");
    drawViewer();
  } catch (error) {
    body.replaceChildren();
    $("#sessions-table").hidden = true;
    $("#sessions-more").hidden = true;
    empty.hidden = false;
    empty.textContent = `Could not read history: ${error.message}`;
  }
}

async function loadSessions() {
  await Promise.all([loadLive(), loadHistory()]);
}

// Kept current only while it is being looked at.
function watchSessions(visible) {
  clearInterval(sessions.timer);
  sessions.timer = null;
  if (!visible) return;
  loadSessions();
  sessions.ticks = 0;
  sessions.timer = setInterval(() => {
    if (document.hidden) return;
    sessions.ticks += 1;
    loadLive();
    if (sessions.ticks % 3 === 0) loadHistory();
  }, SESSIONS_EVERY_MS);
}

function startSessions() {
  $("#sessions-window").replaceChildren(
    ...WINDOWS.map(([slug, label]) => {
      const option = el("option", "", label);
      option.value = slug;
      option.selected = slug === "last_7d";
      return option;
    }),
  );
  const fresh = () => {
    sessions.limit = 0;
    routeChanged();
    loadHistory();
  };
  for (const id of ["#sessions-window", "#sessions-status"]) $(id).addEventListener("change", fresh);
  $("#sessions-agent").addEventListener("input", fresh);
  $("#sessions-more").addEventListener("click", () => loadHistory(true));
  $("#watch-live").addEventListener("click", backToLive);
  $("#sessions-thinking").addEventListener("change", (event) => {
    $("#terminals").classList.toggle("hide-think", !event.target.checked);
  });

  // The session picked is part of the address, so a link to one opens it.
  routeExtras.sessions = (params) => {
    if (!params) return { run: sessions.picked ?? "" };
    const wanted = params.get("run");
    if (!wanted) {
      if (sessions.picked) backToLive();
      return undefined;
    }
    if (wanted !== sessions.picked) {
      get(`/runs/${encodeURIComponent(wanted)}`).then(pick).catch(backToLive);
    }
    return undefined;
  };
}
