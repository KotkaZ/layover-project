// Chains: what a person actually asked for, and what became of it.
//
// A run is one step of a chain. Chains get a list of their own because the interesting failure —
// a chain that stopped with every run reporting success — is invisible in a list of runs.
//
// A real factory starts hundreds a week, almost all of them a schedule's routine work that
// finished, so the list opens on what needs a person — working, waiting for an answer, stalled,
// halted — and is searched and paged in the page rather than drawn whole: 830 rows is a page
// nobody reads to the bottom of.
//
// Uses app.js's helpers (`get`, `el`, `$`, `money`, `whenCell`, `workflowCell`, `STATES`,
// `scope`, `pipelinesByName`) and chain.js's and reply.js's `openChain` and `openReplyFor`.

const chainsView = { all: [], shown: 0, stalled: 0, awaiting: 0 };
const CHAINS_PAGE = 50;
const ATTENTION = new Set(["working", "awaiting_human", "stalled", "halted"]);

/// A chain's state, and for one that is working, where it is.
function chainState(chain) {
  const state = STATES[chain.state] ?? chain.state;
  if (chain.state !== "working") return state;
  if (chain.running) return `${state} · at ${chain.running.join(", ")}`;
  if (chain.queued) return `${state} · queued`;
  return state;
}

/// Routine work: a schedule's chain nobody named, that finished. What hiding them hides.
function routine(chain) {
  if (chain.state !== "finished" || chain.name) return false;
  const trigger = pipelinesByName.get(chain.pipeline)?.trigger;
  return Boolean(trigger) && trigger.kind !== "manual";
}

/// The chains that pass the page's filters, newest first as the Tower lists them.
function chainsShown() {
  const state = $("#chains-state").value;
  const needle = $("#chains-search").value.trim().toLowerCase();
  const hideRoutine = $("#chains-routine").checked;
  return chainsView.all.filter((chain) => {
    // The header's workflow narrows this list as it narrows every other.
    if (scope() && chain.pipeline !== scope()) return false;
    if (state === "attention" && !ATTENTION.has(chain.state)) return false;
    if (state && state !== "attention" && chain.state !== state) return false;
    if (hideRoutine && routine(chain)) return false;
    if (!needle) return true;
    return [chain.name, chain.pipeline, chain.itinerary_id, ...(chain.agents ?? [])]
      .filter(Boolean)
      .some((text) => text.toLowerCase().includes(needle));
  });
}

function chainRow(chain) {
  const row = el("tr");
  const cost = chain.measured === false ? `${money(chain.usd)}+` : money(chain.usd);
  row.append(
    whenCell(chain.started_at),
    workflowCell(chain.pipeline, chain.name),
    el("td", "", (chain.agents ?? []).join(" → ") || "—"),
    el("td", "num", String(chain.runs)),
    el("td", "num", cost),
    el("td", `outcome ${chain.state}`, chainState(chain)),
  );

  // Only what needs a person here: a chain waiting for an answer gets it. Continuing a chain is
  // a new piece of work, and lives on the chain's own page rather than on every finished row.
  const actions = el("td", "actions");
  if (chain.waiting_for) {
    const answer = el("button", "link", "Reply…");
    answer.title = `${chain.waiting_for.agent} asked: ${chain.waiting_for.summary}`;
    answer.addEventListener("click", () => openReplyFor(chain.waiting_for.run_id));
    actions.append(answer);
  }
  actions.append(el("span", "open", "›"));
  row.append(actions);

  // The reason lives in a tooltip rather than a column: it is a sentence, and a column wide
  // enough for it would squeeze out everything that is scannable.
  row.title = chain.detail ? `${chain.detail} — open this chain` : "Open this chain";
  row.classList.add("readable", "opens");
  row.addEventListener("click", (event) => {
    if (event.target.closest("button")) return;
    openChain(chain.itinerary_id);
  });
  return row;
}

/// Draws the filtered chains, a page at a time. `more` adds a page rather than starting again.
function showChains(more = false) {
  const body = $("#chains-table tbody");
  const shown = chainsShown();
  chainsView.shown = more ? chainsView.shown + CHAINS_PAGE : CHAINS_PAGE;
  if (!more) body.replaceChildren();
  const from = body.children.length;
  for (const chain of shown.slice(from, chainsView.shown)) body.append(chainRow(chain));

  const total = chainsView.all.length;
  const attention = chainsView.all.filter((chain) => ATTENTION.has(chain.state)).length;
  const parts = [`${total} chain(s)`];
  if (attention) parts.push(`${attention} need attention`);
  if (chainsView.stalled) parts.push(`${chainsView.stalled} stalled`);
  if (chainsView.awaiting) parts.push(`${chainsView.awaiting} waiting for you`);
  if (shown.length !== total) parts.push(`${shown.length} shown`);
  $("#chains-count").textContent = total === 0 ? "Nothing ran in this window." : parts.join(" · ");

  $("#chains-table").hidden = shown.length === 0;
  const more_ = $("#chains-more");
  const left = shown.length - body.children.length;
  more_.hidden = left <= 0;
  more_.textContent = `Show ${Math.min(left, CHAINS_PAGE)} more of ${left}`;

  const empty = $("#chains-empty");
  empty.replaceChildren();
  empty.hidden = shown.length > 0 || total === 0;
  if (!empty.hidden) {
    empty.append(
      $("#chains-state").value === "attention"
        ? `Nothing needs attention — all ${total} chain(s) in this window finished. `
        : "No chain matches. ",
    );
    const all = el("button", "link", "Show every chain");
    all.type = "button";
    all.addEventListener("click", () => {
      $("#chains-state").value = "";
      $("#chains-search").value = "";
      $("#chains-routine").checked = false;
      routeChanged();
      showChains();
    });
    empty.append(all);
  }
}

async function loadChains() {
  const params = new URLSearchParams({ window: $("#chains-window").value });

  try {
    const { itineraries, stalled, awaiting_human: awaiting } = await get(`/itineraries?${params}`);
    chainsView.all = itineraries;
    chainsView.stalled = stalled;
    chainsView.awaiting = awaiting;
    // A refresh keeps however many pages somebody had opened, so they do not lose their place.
    const pages = Math.max(1, Math.ceil(chainsView.shown / CHAINS_PAGE));
    showChains();
    for (let page = 1; page < pages; page += 1) showChains(true);

    const badge = $("#stalled-badge");
    badge.textContent = String(stalled);
    badge.hidden = stalled === 0;
    const waiting = $("#awaiting-badge");
    waiting.textContent = String(awaiting);
    waiting.hidden = awaiting === 0;
  } catch (error) {
    $("#chains-table tbody").replaceChildren();
    $("#chains-table").hidden = true;
    $("#chains-more").hidden = true;
    $("#chains-count").textContent = `Could not read chains: ${error.message}`;
  }
}

function startChainsList() {
  $("#chains-window").addEventListener("change", () => {
    routeChanged();
    loadChains();
  });
  for (const id of ["#chains-state", "#chains-routine"]) {
    $(id).addEventListener("change", () => {
      routeChanged();
      showChains();
    });
  }
  $("#chains-search").addEventListener("input", () => {
    routeChanged();
    showChains();
  });
  $("#chains-more").addEventListener("click", () => showChains(true));
}
