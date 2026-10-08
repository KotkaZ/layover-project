// One chain, whole: its workflow drawn for it alone, every run in the order it happened, and what
// it is waiting for.
//
// A workflow's route map is one drawing however many times the workflow was triggered, so with
// three development runs going at once it says "the coder is running" and not which of the three
// is where. This view answers that for one chain at a time, and keeps answering while it works.
//
// Separate from app.js, which is already long; it uses that file's helpers (`get`, `el`, `when`,
// `duration`, `money`, `showView`) and the session and reply windows, all of which exist by the
// time anything here is called.

const chainView = { id: null, timer: null, svg: null };

/// Opens one chain, and keeps it current while it works.
async function openChain(id) {
  chainView.id = id;
  chainView.svg = null;
  $("#chain-canvas").replaceChildren(el("p", "empty", "drawing\u2026"));
  showView("chain");
  // Linkable, and kept across a reload: a chain is the thing somebody wants to send a colleague.
  history.replaceState(null, "", `#chain=${encodeURIComponent(id)}`);
  await drawChain();
}

/// Called whenever the visible view changes. Polling stops the moment the view is left.
function watchChain(visible) {
  clearInterval(chainView.timer);
  chainView.timer = null;
  if (visible) {
    chainView.timer = setInterval(() => {
      if (!document.hidden) drawChain();
    }, 3000);
    return;
  }
  chainView.id = null;
  if (location.hash.startsWith("#chain=")) {
    history.replaceState(null, "", location.pathname + location.search);
  }
}

/// Who started a run, as its record says: an agent, the way in, or nobody knows.
function sentBy(run) {
  if (run.sent_by === null || run.sent_by === undefined) return ["—", "not recorded"];
  if (run.sent_by.length === 0) return ["way in", "a trigger, a schedule or a resumed layover"];
  return [run.sent_by.join(", "), ""];
}

function chainHeader(chain, runs) {
  $("#chain-title").replaceChildren(
    ...(chain.name ? [el("span", "", chain.name)] : []),
    el("span", chain.name ? "muted" : "", chain.pipeline ?? "no workflow"),
    el("code", "", chain.itinerary_id),
  );
  const state = $("#chain-state");
  state.className = `outcome ${chain.state}`;
  state.textContent = STATES[chain.state] ?? chain.state;

  const ended = chain.finished_at ? new Date(chain.finished_at) : new Date();
  const took = Math.max(0, Math.round((ended - new Date(chain.started_at)) / 1000));
  const cost = chain.measured === false ? `${money(chain.usd)}+` : money(chain.usd);
  const facts = [
    `started ${when(chain.started_at)}`,
    `${chain.finished_at ? "took" : "for"} ${duration(took)}`,
    `${runs.length} run(s)`,
    cost,
  ];
  if (chain.running) facts.push(`at ${chain.running.join(", ")}`);
  if (chain.queued) facts.push(`queued for ${chain.queued.join(", ")}`);
  $("#chain-facts").textContent = facts.join(" · ");

  const flags = Object.entries(chain.flags ?? {});
  $("#chain-flags").textContent = flags.length
    ? `Flags: ${flags.map(([name, on]) => `${name}=${on}`).join(", ")}`
    : "";

  const detail = $("#chain-detail");
  detail.hidden = !chain.detail;
  detail.textContent = chain.detail ?? "";

  const actions = $("#chain-actions");
  actions.replaceChildren();
  if (chain.waiting_for) {
    const answer = el("button", "link", "Reply…");
    answer.title = `${chain.waiting_for.agent} asked: ${chain.waiting_for.summary}`;
    answer.addEventListener("click", () => openReplyFor(chain.waiting_for.run_id));
    actions.append(answer);
  }
  if (chain.pipeline) {
    const onward = el("button", "link", "Continue…");
    onward.title = "Trigger this workflow again, with this chain's flags.";
    onward.addEventListener("click", () =>
      continueChain(chain.pipeline, chain.flags ?? null, chain.itinerary_id, chain.name),
    );
    actions.append(onward);
  }
  for (const [label, other] of [
    ["Continues", chain.continues ? [chain.continues] : []],
    ["Continued by", chain.continued_by ?? []],
  ]) {
    for (const id of other) {
      const link = el("button", "link", `${label} ${id.slice(-6)}`);
      link.title = id;
      link.addEventListener("click", () => openChain(id));
      actions.append(link);
    }
  }
}

/// The map, redrawn only when it changed: a redraw every few seconds would drop whatever agent
/// somebody had clicked to read about.
function chainMap(map) {
  if (map.mermaid === chainView.svg) return;
  chainView.svg = map.mermaid;
  const canvas = $("#chain-canvas");
  // The panel the last drawing's tracing added beside it; the new drawing brings its own.
  const panel = canvas.nextElementSibling;
  if (panel?.classList.contains("trace")) panel.remove();
  // Every map names its arrowheads `#arrow`, and a reference finds the first in the page — which
  // is in the hidden route map view while this one is showing, and a hidden marker draws nothing.
  canvas.innerHTML = map.mermaid
    .replaceAll('id="arrow', 'id="chain-arrow')
    .replaceAll("url(#arrow", "url(#chain-arrow");
  canvas.querySelector("svg.routemap")?.classList.add("chain");
  traceable(canvas, {
    describe: (name) => agentsByName.get(name),
    showRuns: (name) => {
      $("#runs-agent").value = name;
      showView("runs");
    },
  });
}

function chainRuns(runs, pending) {
  const body = $("#chain-runs tbody");
  body.replaceChildren();

  runs.forEach((run, index) => {
    const row = el("tr");
    const [from, why] = sentBy(run);
    const fromCell = el("td", run.sent_by?.length ? "" : "muted", from);
    if (why) fromCell.title = why;
    row.append(
      el("td", "num muted", String(index + 1)),
      el("td", "", when(run.started_at)),
      el("td", "", run.agent),
      fromCell,
      el("td", `outcome ${run.status}`, run.status.replace("_", " ")),
      el("td", "num", duration(run.duration_sec)),
      el(
        "td",
        "num",
        run.status === "running" ? "—" : run.cost_usd === null ? "not reported" : money(run.cost_usd),
      ),
    );
    const actions = el("td", "actions");
    const watch = el("button", "link", run.status === "running" ? "Watch" : "Transcript");
    watch.addEventListener("click", () => openSession(run));
    const report = el("button", "link", "Report");
    report.addEventListener("click", () => openReport(run.run_id, run));
    actions.append(watch, report);
    row.append(actions);
    if (run.detail) row.title = run.detail;
    body.append(row);
  });

  // What comes next, after what has happened.
  for (const flight of pending) {
    const row = el("tr", "pending");
    row.append(
      el("td", "num muted", "…"),
      el("td", "", when(flight.queued_at)),
      el("td", "", flight.to),
      el("td", "muted", "—"),
      el("td", "outcome queued", "queued"),
      el("td", "num muted", "waiting for a slot"),
      el("td", "num", "—"),
      el("td", ""),
    );
    body.append(row);
  }

  $("#chain-empty").hidden = runs.length + pending.length > 0;
}

async function drawChain() {
  const id = chainView.id;
  if (!id) return;
  try {
    const { itinerary: chain, runs, pending, map } = await get(
      `/itineraries/${encodeURIComponent(id)}`,
    );
    // Another chain was opened while this one was being read.
    if (id !== chainView.id) return;
    $("#chain-error").hidden = true;
    chainHeader(chain, runs);
    chainMap(map);
    chainRuns(runs, pending);
    // Nothing more will happen to a chain that has stopped, so there is nothing to keep asking.
    if (chain.state !== "working") {
      clearInterval(chainView.timer);
      chainView.timer = null;
    }
  } catch (error) {
    if (id !== chainView.id) return;
    $("#chain-error").hidden = false;
    $("#chain-error").textContent = `Could not read the chain: ${error.message}`;
  }
}

/// A row of the chains a workflow has going, under its map: one button each, saying where it is.
function inFlight(chains) {
  const strip = el("div", "chips");
  for (const chain of chains) {
    const chip = el("button", `chip ${chain.state}`);
    chip.type = "button";
    const where = chain.running
      ? `at ${chain.running.join(", ")}`
      : chain.queued
        ? `queued for ${chain.queued.join(", ")}`
        : STATES[chain.state] ?? chain.state;
    chip.append(
      chain.name ? el("b", "", chain.name) : el("code", "", chain.itinerary_id.slice(-6)),
      document.createTextNode(` ${where}`),
    );
    chip.title = `${chain.itinerary_id} · started ${when(chain.started_at)} · ${chain.runs} run(s)`;
    chip.addEventListener("click", () => openChain(chain.itinerary_id));
    strip.append(chip);
  }
  return strip;
}

function startChains() {
  $("#chain-back").addEventListener("click", () => showView("chains"));
  $("#chain-refresh").addEventListener("click", () => {
    chainView.svg = null;
    drawChain();
  });
  window.addEventListener("hashchange", () => {
    const wanted = decodeURIComponent(location.hash.replace(/^#chain=/, ""));
    if (location.hash.startsWith("#chain=") && wanted !== chainView.id) openChain(wanted);
  });
}

/// The chain a link or a reload asked for, if any.
function chainInAddress() {
  return location.hash.startsWith("#chain=")
    ? decodeURIComponent(location.hash.slice("#chain=".length))
    : null;
}
