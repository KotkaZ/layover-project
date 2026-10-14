// Runs: every supervised CLI execution, live and historical.
//
// Paged rather than drawn whole, a hundred at a time, because a busy factory runs a thousand a
// week. Each row says which chain it belongs to — by name where the chain has one — and what the
// run actually ran on, since "which effort did that run use?" is a question history can answer.
//
// Uses app.js's helpers (`get`, `el`, `$`, `money`, `duration`, `whenCell`, `workflowCell`,
// `scoped`, `openReport`) and chain.js's and sessions.js's `openChain` and `openSession`.

const runsView = { limit: 0 };
const RUNS_PAGE = 100;

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
  const cell = el("td", `outcome ${run.status}`, run.status.replace("_", " "));
  if (run.cause === "provider_error") {
    cell.append(el("span", "cause", " · model service"));
    cell.title = `The model service failed a call and the run ended there${run.request_id ? ` (request ${run.request_id})` : ""}. ${run.detail ?? ""}`.trim();
  }
  return cell;
}

function runRow(run) {
  const row = el("tr");
  // A run still going has not finished: say when it started, and that it is still going.
  row.append(
    run.status === "running"
      ? whenCell(run.started_at, "running · started ")
      : whenCell(run.finished_at ?? run.started_at),
    el("td", "", run.agent),
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
  open.title = `${run.chain_name ? `${run.chain_name} — ` : ""}${run.itinerary_id}: see this chain whole`;
  open.addEventListener("click", (event) => {
    event.stopPropagation();
    openChain(run.itinerary_id);
  });
  chain.append(open);

  // How it got there, as its CLI printed it: live for a run still going, a replay otherwise.
  const watching = el("td", "actions");
  const transcript = el("button", "link", run.status === "running" ? "Watch" : "Transcript");
  transcript.type = "button";
  transcript.addEventListener("click", (event) => {
    event.stopPropagation();
    openSession(run);
  });
  watching.append(transcript);
  row.append(chain, watching);

  if (run.detail) row.title = run.detail;
  // Every row opens whatever the agent wrote about itself.
  row.classList.add("readable");
  row.addEventListener("click", () => openReport(run.run_id, run));
  return row;
}

/// Loads the newest runs; `more` asks for the next page as well.
async function loadRuns(more = false) {
  const body = $("#runs-table tbody");
  const empty = $("#runs-empty");
  runsView.limit = more ? runsView.limit + RUNS_PAGE : Math.max(runsView.limit, RUNS_PAGE);
  const params = new URLSearchParams({
    window: $("#runs-window").value,
    limit: String(runsView.limit + 1),
  });
  if ($("#runs-status").value) params.set("status", $("#runs-status").value);
  if ($("#runs-agent").value.trim()) params.set("agent", $("#runs-agent").value.trim());
  scoped(params);

  try {
    const { runs } = await get(`/runs?${params}`);
    // One more than shown is asked for, so whether there is another page is known without a
    // second request.
    const hasMore = runs.length > runsView.limit;
    body.replaceChildren(...runs.slice(0, runsView.limit).map(runRow));

    empty.hidden = runs.length > 0;
    empty.textContent = $("#runs-status").value || $("#runs-agent").value.trim()
      ? "No run in this window matches."
      : "Nothing ran in this window.";
    $("#runs-table").hidden = runs.length === 0;
    const button = $("#runs-more");
    button.hidden = !hasMore;
    button.textContent = `Show ${RUNS_PAGE} more`;
    updated("#runs-updated");
  } catch (error) {
    body.replaceChildren();
    $("#runs-table").hidden = true;
    $("#runs-more").hidden = true;
    empty.hidden = false;
    empty.textContent = `Could not read history: ${error.message}`;
  }
}

function startRuns() {
  const fresh = () => {
    runsView.limit = 0;
    routeChanged();
    loadRuns();
  };
  for (const id of ["#runs-window", "#runs-status"]) $(id).addEventListener("change", fresh);
  $("#runs-agent").addEventListener("input", fresh);
  $("#runs-more").addEventListener("click", () => loadRuns(true));
}
