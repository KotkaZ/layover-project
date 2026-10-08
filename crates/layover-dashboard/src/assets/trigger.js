// Triggering a workflow: the one control on the page that starts work.
//
// What it queues is a flight, which the Tower starts within seconds; the window says what was
// queued rather than pretending it has already run. Besides the prompt and the workflow's flags, a
// person may name the run — "Login page: retry banner" — so it can be found among three others of
// the same workflow, and choose a different model, effort or context for any agent the workflow
// runs, for this run only. Both stay with everything the run causes.
//
// Separate from app.js, which is already long; it uses that file's helpers (`get`, `send`, `el`,
// `$`, `pipelinesByName`, `openChain`, `loadQueued`), all of which exist by the time anything here
// is called.

function showFlags(name) {
  const box = $("#trigger-flags");
  const pipeline = pipelinesByName.get(name);
  box.querySelectorAll(".flag").forEach((node) => node.remove());

  const flags = pipeline?.flags ?? [];
  $("#trigger-noflags").hidden = flags.length > 0;

  for (const flag of flags) {
    const row = el("label", "flag");
    const toggle = document.createElement("input");
    toggle.type = "checkbox";
    toggle.dataset.flag = flag.name;
    // Start from the declared default, so the window shows what would happen if you changed
    // nothing rather than a set of switches that all read false.
    toggle.checked = flag.default;

    const text = el("span");
    text.append(el("span", "n", flag.name));
    if (flag.description) text.append(document.createElement("br"), el("span", "d", flag.description));

    row.append(toggle, text);
    box.append(row);
  }
}

/// Every value some agent of the factory runs at, for suggesting — never for checking. The CLI
/// knows which values a model accepts; this only saves typing one already in use.
function suggestions() {
  const seen = { model: new Set(), effort: new Set(), context: new Set() };
  for (const pipeline of pipelinesByName.values()) {
    for (const agent of pipeline.agents ?? []) {
      if (agent.model) seen.model.add(agent.model);
      if (agent.reasoning_effort) seen.effort.add(agent.reasoning_effort);
      if (agent.context) seen.context.add(agent.context);
    }
  }
  for (const [key, values] of Object.entries(seen)) {
    $(`#trigger-${key}s`).replaceChildren(
      ...[...values].sort().map((value) => {
        const option = el("option");
        option.value = value;
        return option;
      }),
    );
  }
}

// One row per agent the workflow runs: what it runs at now, and a box to choose otherwise for this
// run. A value its runner has no placeholder for cannot reach its CLI, so that box is disabled
// rather than offered and silently ignored.
function showChoices(name) {
  const pipeline = pipelinesByName.get(name);
  const agents = pipeline?.agents ?? [];
  const body = $("#trigger-agents tbody");
  body.replaceChildren();
  $("#trigger-agents").hidden = agents.length === 0;

  for (const agent of agents) {
    const row = el("tr");
    row.dataset.agent = agent.agent;
    row.append(el("td", "", agent.agent));
    for (const [key, now, takes] of [
      ["model", agent.model, agent.takes_model],
      ["effort", agent.reasoning_effort, agent.takes_effort],
      ["context", agent.context, agent.takes_context],
    ]) {
      const input = document.createElement("input");
      input.type = "text";
      input.dataset.key = key;
      input.setAttribute("list", `trigger-${key}s`);
      input.placeholder = now ?? "CLI default";
      input.disabled = !takes;
      input.title = takes
        ? `Leave empty to run ${agent.agent} at ${now ?? "its CLI's default"}.`
        : `${agent.agent}'s runner has no {${key}} placeholder, so a ${key} chosen here could not reach its CLI.`;
      const cell = el("td");
      cell.append(input);
      row.append(cell);
    }
    body.append(row);
  }
  $("#trigger-agents").open = false;
}

// `preset` continues a chain: its workflow selected, its flags set and its name kept, saying where
// they came from. Without one the dialog starts from the workflow's defaults, which is what a fresh
// trigger means.
async function openTrigger(preset) {
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
  $("#trigger-name").value = preset?.name ?? "";
  const origin = $("#trigger-origin");
  origin.hidden = true;
  if (preset?.pipeline && pipelinesByName.has(preset.pipeline)) select.value = preset.pipeline;
  showFlags(select.value);
  showChoices(select.value);
  suggestions();

  if (preset?.from) {
    origin.hidden = false;
    if (preset.flags) {
      for (const box of $("#trigger-flags").querySelectorAll("input[type=checkbox]")) {
        if (box.dataset.flag in preset.flags) box.checked = preset.flags[box.dataset.flag];
      }
      origin.className = "hint";
      origin.textContent = `Continuing chain ${preset.from}: the workflow, flags and name are set as that chain had them. Change them if you need to.`;
    } else {
      origin.className = "caveat";
      origin.textContent = `Chain ${preset.from} did not record its flags — it ran before they were kept. These are ${select.value}'s defaults: set them as it had them.`;
    }
  }

  const { dispatched_by: by } = await get("/flights").catch(() => ({ dispatched_by: null }));
  $("#trigger-note").textContent = by
    ? `Queued work is started by ${by} as soon as a slot is free.`
    : "This queues the work. Nothing in this process starts it — the dashboard is watching only — so it waits until a Tower (`layover serve`) or `layover run` picks it up.";

  $("#trigger").showModal();
}

/// The agents a person chose differently for, and only what they chose.
function chosenAgents() {
  const agents = {};
  for (const row of $("#trigger-agents tbody").querySelectorAll("tr")) {
    const choice = {};
    for (const input of row.querySelectorAll("input")) {
      const value = input.value.trim();
      if (value && !input.disabled) choice[input.dataset.key] = value;
    }
    if (Object.keys(choice).length > 0) agents[row.dataset.agent] = choice;
  }
  return agents;
}

async function submitTrigger(event) {
  const pipeline = $("#trigger-pipeline").value;
  const body = $("#trigger-body").value.trim();
  const name = $("#trigger-name").value.trim();

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
    const accepted = await send("/flights", "POST", {
      pipeline,
      body,
      flags,
      name: name || null,
      agents: chosenAgents(),
    });
    $("#trigger").close();
    loadQueued();
    // What somebody who has just triggered a workflow wants next is to watch that run of it.
    if (accepted.itinerary_id) openChain(accepted.itinerary_id);
  } catch (error) {
    $("#trigger-error").hidden = false;
    $("#trigger-error").textContent = `Not queued: ${error.message}`;
  }
}

function startTrigger() {
  $("#trigger-open").addEventListener("click", () => openTrigger());
  $("#trigger-pipeline").addEventListener("change", (e) => {
    // Another workflow is not the chain being continued, so its flags say nothing about it.
    $("#trigger-origin").hidden = true;
    showFlags(e.target.value);
    showChoices(e.target.value);
  });
  $("#trigger-send").addEventListener("click", submitTrigger);
}
