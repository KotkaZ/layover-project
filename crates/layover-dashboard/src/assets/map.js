// The route map: one card per workflow — what it is wired to do, what it did this week, and the
// chains of it under way — kept current while it is on screen.
//
// One diagram per workflow. Drawn together they read as a single very confused process, and a
// selector still only lets you see one at a time without being able to compare them. A factory
// with five workflows is then five drawings tall, so a drawing folds away to its card, and the
// busiest workflows come first.
//
// Uses app.js's helpers (`get`, `el`, `$`, `money`, `spend`, `scope`, `agentsByName`, `showView`),
// routemap.js's `traceable`, chain.js's `inFlight`, switch.js's `workflowSwitch` and `offRail`,
// trigger.js's `startable` and `openTrigger`, and upcoming.js's `clockTime` and `fromNow`.

const mapView = { order: [], drawn: new Map(), generation: 0 };
const FOLDS = "layover.map.folded";

function folds() {
  try {
    return JSON.parse(localStorage.getItem(FOLDS)) ?? {};
  } catch {
    return {};
  }
}

function keepFold(name, folded) {
  const all = folds();
  all[name] = folded;
  try {
    localStorage.setItem(FOLDS, JSON.stringify(all));
  } catch {
    // A browser that keeps nothing still folds; it only forgets on reload.
  }
}

/// Whether a workflow's drawing starts folded away. What somebody chose in this browser wins;
/// otherwise a drawing is open when there is something in it to look at — work under way, a
/// failure, a question — or when there are too few workflows on the page for folding to help.
function startsFolded(name, facts, few) {
  const chosen = folds()[name];
  if (chosen !== undefined) return chosen;
  if (few) return false;
  return !(facts.going > 0 || facts.stats.failed > 0 || facts.stats.help > 0);
}

/// Busiest first: chains under way, then the last week's runs. The configuration's order breaks
/// ties, so a quiet factory reads as it was written.
function byActivity(pipelines, activity, going) {
  const weight = (pipeline) => [
    going.filter((chain) => chain.pipeline === pipeline.name).length,
    activity.get(pipeline.name)?.runs ?? 0,
  ];
  return pipelines
    .map((pipeline, index) => ({ pipeline, index, weight: weight(pipeline) }))
    .sort((a, b) => b.weight[0] - a.weight[0] || b.weight[1] - a.weight[1] || a.index - b.index)
    .map(({ pipeline }) => pipeline);
}

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


const NO_ACTIVITY = { runs: 0, usd: 0, unreported_runs: 0, failed: 0, help: 0 };

/// A workflow's card header: its name, its switch, what it is for, its rails and its trigger.
function workflowHeader(pipeline, scheduled, now, folded) {
  const header = el("header");
  const fold = el("button", "fold", folded ? "▸" : "▾");
  fold.type = "button";
  fold.setAttribute("aria-expanded", String(!folded));
  fold.title = folded ? "Show its drawing" : "Fold its drawing away";
  fold.addEventListener("click", () => {
    setFolded(pipeline.name, !fold.closest(".workflow").classList.contains("folded"), true);
  });
  header.append(fold, el("h2", "", pipeline.name), workflowSwitch(pipeline));
  if (pipeline.description) {
    const what = el("span", "what", pipeline.description);
    what.title = pipeline.description;
    header.append(what);
  }

  const rails = el("div", "rails");
  rails.append(rail("trigger", "", describeTrigger(pipeline.trigger)));
  // Off, a schedule's next tick does not fire, so it is not offered as when it will.
  const next = pipeline.switched_off ? offRail(pipeline) : nextRail(scheduled, now);
  if (next) rails.append(next);
  rails.append(
    // The two rails that bound a chain, and they bound different things: Hops is depth, Fuel is
    // breadth. Showing one without the other invites the assumption that Hops caps spending,
    // which it does not.
    rail("hops", "hops", `${pipeline.max_hops}`),
    rail("fuel", "fuel", money(pipeline.fuel_usd)),
    rail("", "workspace", pipeline.workspace),
  );
  if (pipeline.resumes) rails.append(rail("", "", "resumes layovers"));
  header.append(rails);
  // Started from where it is drawn. A workflow with no entry only resumes layovers, and has
  // nothing a person could start.
  if (startable(pipeline.name)) {
    const go = el("button", "btn", "Trigger…");
    go.type = "button";
    go.title = `Start ${pipeline.name}: its first run is ${pipeline.entry}.`;
    go.addEventListener("click", () => openTrigger({ pipeline: pipeline.name }));
    header.append(go);
  }
  return header;
}

/// Folds a workflow's drawing away or brings it back; `chosen` keeps the choice for next time.
function setFolded(name, folded, chosen) {
  const section = document.querySelector(`#workflows .workflow[data-workflow="${CSS.escape(name)}"]`);
  if (!section) return;
  section.classList.toggle("folded", folded);
  const fold = section.querySelector("header .fold");
  fold.textContent = folded ? "▸" : "▾";
  fold.setAttribute("aria-expanded", String(!folded));
  fold.title = folded ? "Show its drawing" : "Fold its drawing away";
  if (chosen) keepFold(name, folded);
}

/// Draws a workflow's map into its canvas — unless it is the drawing already there, or somebody
/// is tracing an agent on it, since a redraw would drop what they had lit. That one waits for the
/// next refresh.
function drawGraph(name, canvas, svg) {
  if (mapView.drawn.get(name) === svg) return;
  if (canvas.querySelector("svg.routemap.tracing")) return;
  mapView.drawn.set(name, svg);
  // The panel the last drawing's tracing added beside it; the new drawing brings its own.
  const panel = canvas.nextElementSibling;
  if (panel?.classList.contains("trace")) panel.remove();
  canvas.innerHTML = svg;
  traceable(canvas, {
    describe: (agent) => agentsByName.get(agent),
    showRuns: (agent) => {
      $("#runs-agent").value = agent;
      showView("runs");
    },
  });
}

/// Reads everything the cards say, in five requests however many workflows there are, plus one
/// drawing each.
async function mapFacts() {
  const { pipelines } = await get("/pipelines");
  const showing = scope() ? pipelines.filter((p) => p.name === scope()) : pipelines;
  const [activity, going, upcoming, graphs] = await Promise.all([
    activityByWorkflow(),
    chainsGoing(),
    get("/upcoming").catch(() => null),
    Promise.all(
      showing.map((pipeline) =>
        get(`/graph?pipeline=${encodeURIComponent(pipeline.name)}`).catch((error) => ({ error })),
      ),
    ),
  ]);
  return {
    pipelines,
    showing,
    activity,
    going,
    scheduled: new Map((upcoming?.workflows ?? []).map((w) => [w.pipeline, w])),
    now: upcoming ? new Date(upcoming.now) : new Date(),
    graphs: new Map(showing.map((pipeline, i) => [pipeline.name, graphs[i]])),
  };
}

/// Fills one workflow's card from what was read: everything but the drawing is replaced, and the
/// drawing only when it changed.
function fillCard(section, pipeline, facts) {
  const folded = section.classList.contains("folded");
  section.classList.toggle("off", pipeline.switched_off);
  const stats = facts.activity.get(pipeline.name) ?? NO_ACTIVITY;
  const canvas = section.querySelector(".canvas");
  const parts = [
    workflowHeader(pipeline, facts.scheduled.get(pipeline.name), facts.now, folded),
    summaryStrip(pipeline.name, stats, facts.scheduled.get(pipeline.name)?.skipped_7d ?? null),
  ];
  // One map per workflow, however many times it was triggered: these say which run is where.
  const mine = facts.going.filter((chain) => chain.pipeline === pipeline.name);
  if (mine.length) parts.push(inFlight(mine));
  section.querySelectorAll(":scope > :not(.canvas):not(.trace)").forEach((node) => node.remove());
  canvas.before(...parts);

  const map = facts.graphs.get(pipeline.name);
  if (map?.error) {
    mapView.drawn.delete(pipeline.name);
    canvas.replaceChildren(el("p", "empty", `Could not draw it: ${map.error.message}`));
  } else if (map) {
    drawGraph(pipeline.name, canvas, map.mermaid);
    if (map.config_path) $("#config-path").textContent = map.config_path;
  }
}

/// Draws the route map. `quiet` refreshes the cards already on the page, in the order they are
/// in, rather than starting again — so a card does not jump while somebody is reading it, and a
/// traced agent stays lit. It starts again only when the workflows themselves changed.
async function loadMap(quiet = false) {
  // A later draw supersedes this one: the workflow selector changed while it was reading, say.
  const generation = ++mapView.generation;
  const host = $("#workflows");
  if (host.childElementCount === 0) host.append(el("p", "empty", "drawing\u2026"));
  try {
    const facts = await mapFacts();
    if (generation !== mapView.generation) return;
    if (facts.pipelines.length === 0) {
      host.replaceChildren(el("p", "empty", "No pipelines are declared, so nothing can be triggered."));
      mapView.order = [];
      return;
    }

    const names = facts.showing.map((pipeline) => pipeline.name);
    const same = names.length === mapView.order.length && names.every((n) => mapView.order.includes(n));
    if (!quiet || !same) {
      const few = facts.showing.length <= 2;
      host.replaceChildren();
      mapView.drawn.clear();
      for (const pipeline of byActivity(facts.showing, facts.activity, facts.going)) {
        const section = el("section", "workflow");
        section.dataset.workflow = pipeline.name;
        const going = facts.going.filter((chain) => chain.pipeline === pipeline.name).length;
        const stats = facts.activity.get(pipeline.name) ?? NO_ACTIVITY;
        section.classList.toggle("folded", startsFolded(pipeline.name, { going, stats }, few));
        section.append(el("div", "canvas"));
        host.append(section);
      }
      mapView.order = [...host.children].map((section) => section.dataset.workflow);
      $("#map-folds").hidden = facts.showing.length < 2;
    }
    for (const pipeline of facts.showing) {
      const section = host.querySelector(`.workflow[data-workflow="${CSS.escape(pipeline.name)}"]`);
      fillCard(section, pipeline, facts);
    }
    updated("#map-updated");
  } catch (error) {
    if (generation !== mapView.generation) return;
    host.replaceChildren(el("p", "empty", `Could not read the pipelines: ${error.message}`));
    mapView.order = [];
  }
}

function startMap() {
  $("#refresh").addEventListener("click", () => loadMap());
  $("#map-unfold").addEventListener("click", () => {
    for (const name of mapView.order) setFolded(name, false, true);
  });
  $("#map-fold").addEventListener("click", () => {
    for (const name of mapView.order) setFolded(name, true, true);
  });
}
