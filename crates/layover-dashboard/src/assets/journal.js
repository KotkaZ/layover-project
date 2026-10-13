// Help requests and learnings: what agents asked people, and what they worked out.
//
// Learnings are filtered in the page — by agent, by standing and by what they say — because a
// factory accumulates them by the hundred, and the ones worth reading are the few in use. The
// list opens on those; lapsed and rejected ones are a choice away.
//
// Uses app.js's helpers (`get`, `send`, `el`, `$`, `whenCell`, `scoped`, `scope`) and reply.js's
// `openReply`, and upcoming.js's `clockTime`.

const journalView = { learnings: [] };
const STANDING_ORDER = { confirmed: 0, provisional: 1, lapsed: 2, rejected: 3 };

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

function helpRow(request) {
  const row = el("tr");
  const kind = el("td");
  kind.append(el("span", `kind ${request.blocker}`, request.blocker));
  row.append(
    whenCell(request.at),
    el("td", "", request.agent),
    el("td", "", request.pipeline || "\u2014"),
    kind,
  );
  const what = el("td", "wide", request.summary);
  what.title = request.detail;
  row.append(what, el("td", "", request.fatal ? "yes — the run ended" : "no — it carried on"));

  // Reply continues the work with the answer. Resolving says the blocker is gone, not that
  // somebody read this — and after a request that stopped its run there is no next run to
  // notice, so the tooltip says so rather than promising one.
  const action = el("td", "actions");
  const answer = el("button", "link", "Reply");
  answer.title = request.fatal
    ? "Answer it and continue the work: a new run of this agent, with your reply, in the same workflow with the same flags."
    : "Answer it: a new run of this agent with your reply, in the same workflow with the same flags.";
  answer.addEventListener("click", () => openReply(request));
  const done = el("button", "link", "Resolved");
  done.title = request.fatal
    ? "Mark as dealt with. This does not restart anything: the run that asked has ended, and nothing will run until you start it. To continue the work, use Reply."
    : "Mark as dealt with. If it is not, the next run will raise it again.";
  done.addEventListener("click", () => resolveHelp(request.run_id, done));
  action.append(answer, done);
  row.append(action);
  return row;
}

function learningRow(learning) {
  const row = el("tr");
  row.append(el("td", "", learning.agent), el("td", "wide", learning.text));
  row.append(el("td", `standing ${learning.state}`, learning.state));
  const deleted = el("td", "when", learning.deleted_after ? clockTime(learning.deleted_after) : "");
  if (learning.deleted_after) {
    deleted.title = `Deleted after ${new Date(learning.deleted_after).toLocaleString()} unless an agent rediscovers it first.`;
  }
  row.append(
    el("td", "num", `${learning.proposals}\u00d7`),
    el("td", "num", learning.state === "provisional" ? `${learning.runs_left}` : "\u2014"),
    deleted,
  );

  // Not an approval queue: this learning is already being given to runs. These say "keep it for
  // good" and "stop using it", which is a judgement about something in use rather than
  // permission for it to start.
  const action = el("td", "actions");
  if (learning.state !== "confirmed") {
    const keep = el("button", "link", "Keep");
    keep.title = "Confirm it: apply indefinitely instead of lapsing.";
    keep.addEventListener("click", () => judge(learning.id, "confirmed", keep));
    action.append(keep);
  }
  if (learning.state !== "rejected") {
    const drop = el("button", "link danger", "Drop");
    drop.title = "Reject it: stop giving it to runs.";
    drop.addEventListener("click", () => judge(learning.id, "rejected", drop));
    action.append(drop);
  }
  row.append(action);
  row.title = `impact: ${learning.impact}`;
  return row;
}

/// Fills the agent filter with every agent that has learned something, keeping the choice.
function learningAgents() {
  const select = $("#learn-agent");
  // An address can name an agent before the list that holds it has loaded; see router.js.
  const chosen = select.dataset.wanted || select.value;
  delete select.dataset.wanted;
  const agents = [...new Set(journalView.learnings.map((learning) => learning.agent))].sort();
  const any = el("option", "", "any agent");
  any.value = "";
  select.replaceChildren(
    any,
    ...agents.map((agent) => {
      const option = el("option", "", agent);
      option.value = agent;
      return option;
    }),
  );
  if (agents.includes(chosen)) select.value = chosen;
}

/// Counts each standing in the standing filter's labels, so what a choice would show is known
/// before it is made.
function learningCounts() {
  const count = (states) =>
    journalView.learnings.filter((learning) => states.includes(learning.state)).length;
  for (const option of $("#learn-state").options) {
    const states = option.value === "use" ? ["provisional", "confirmed"]
      : option.value === "" ? Object.keys(STANDING_ORDER)
      : [option.value];
    option.textContent = `${option.dataset.label} (${count(states)})`;
  }
}

function showLearnings() {
  const state = $("#learn-state").value;
  const agent = $("#learn-agent").value;
  const needle = $("#learn-search").value.trim().toLowerCase();
  const shown = journalView.learnings
    .filter((learning) => {
      if (state === "use" && !["provisional", "confirmed"].includes(learning.state)) return false;
      if (state && state !== "use" && learning.state !== state) return false;
      if (agent && learning.agent !== agent) return false;
      return !needle || learning.text.toLowerCase().includes(needle);
    })
    // In use first, then by how well established.
    .sort((a, b) => STANDING_ORDER[a.state] - STANDING_ORDER[b.state] || b.proposals - a.proposals);

  $("#learn-table tbody").replaceChildren(...shown.map(learningRow));
  $("#learn-table").hidden = shown.length === 0;
  const empty = $("#learn-empty");
  empty.hidden = shown.length > 0;
  empty.textContent = journalView.learnings.length === 0
    ? "Nothing learned yet."
    : "No learning matches.";
}

async function loadJournal() {
  const helpEmpty = $("#help-empty");
  try {
    const { requests } = await get(
      `/help?${scoped(new URLSearchParams({ open: "true", window: "last_90d" }))}`,
    );
    $("#help-table tbody").replaceChildren(...requests.map(helpRow));
    $("#help-table").hidden = requests.length === 0;
    helpEmpty.hidden = requests.length > 0;
    helpEmpty.textContent = scope() ? `Nothing is stuck in ${scope()}.` : "Nothing is stuck.";
  } catch (error) {
    $("#help-table").hidden = true;
    helpEmpty.hidden = false;
    helpEmpty.textContent = `Could not read help requests: ${error.message}`;
  }

  try {
    const { learnings } = await get("/learnings");
    journalView.learnings = learnings;
    learningAgents();
    learningCounts();
    showLearnings();
  } catch (error) {
    $("#learn-table").hidden = true;
    const empty = $("#learn-empty");
    empty.hidden = false;
    empty.textContent = `Could not read learnings: ${error.message}`;
  }
}

function startJournal() {
  for (const id of ["#learn-state", "#learn-agent"]) {
    $(id).addEventListener("change", () => {
      routeChanged();
      showLearnings();
    });
  }
  $("#learn-search").addEventListener("input", () => {
    routeChanged();
    showLearnings();
  });
}
