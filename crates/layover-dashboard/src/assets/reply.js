// Answering an agent, and picking up a chain where it left off.
//
// A fatal help request ends its chain: "Resolved" restarts nothing, because there is no next run.
// A reply is the way back in — a new run of the agent that asked, told what the person said, in
// the same workflow with the same flags. "Continue…" is the general form: the trigger dialog, set
// to a chain's workflow and flags rather than the workflow's defaults.
//
// Loaded before app.js; uses its helpers ($, el, get, send, when, showFlags) only inside functions.

let replying = null;

function describeFlags(flags) {
  const names = Object.keys(flags);
  if (names.length === 0) return "no flags";
  return names.map((name) => `${name} ${flags[name] ? "on" : "off"}`).join(", ");
}

// Every line of what the agent wrote, quoted, so the answer can go between the questions.
function quoted(detail) {
  return detail
    .split("\n")
    .map((line) => (line.trim() ? `> ${line}` : ">"))
    .join("\n");
}

function openReply(request) {
  replying = request;
  $("#reply-headline").textContent = `Reply to ${request.agent}`;
  $("#reply-meta").textContent =
    `${request.summary} · asked ${when(request.at)}${request.fatal ? " · it stopped to wait for this" : ""}`;
  $("#reply-body").value = `${quoted(request.detail)}\n\n`;
  $("#reply-by").value = localStorage.getItem("layover.replier") ?? "";
  $("#reply-error").hidden = true;

  const pipeline = request.pipeline;
  const declared = pipeline ? (pipelinesByName.get(pipeline)?.flags ?? []) : [];
  const box = $("#reply-flags");
  box.querySelectorAll(".flag").forEach((node) => node.remove());

  if (request.flags) {
    // Recorded: carried as they were, and said so, because this is the decision the reply makes.
    box.hidden = true;
    $("#reply-carries").textContent =
      `${request.agent} gets this as a new run${pipeline ? ` of ${pipeline}` : ""}, with ${describeFlags(request.flags)} — as the chain that asked had them.`;
  } else if (declared.length > 0) {
    // Not recorded: the person has to say. Starting from the defaults is the downgrade a reply
    // exists to avoid, so the window says that is what these are.
    box.hidden = false;
    $("#reply-flags-warning").textContent =
      `Layover did not record this chain's flags — it asked before they were kept. Set them as it had them; these start from ${pipeline}'s defaults.`;
    for (const flag of declared) {
      const row = el("label", "flag");
      const input = document.createElement("input");
      input.type = "checkbox";
      input.dataset.flag = flag.name;
      input.checked = flag.default;
      row.append(input, el("span", "n", flag.name));
      box.append(row);
    }
    $("#reply-carries").textContent = `${request.agent} gets this as a new run of ${pipeline}.`;
  } else {
    box.hidden = true;
    $("#reply-carries").textContent =
      `${request.agent} gets this as a new run${pipeline ? ` of ${pipeline}` : ""}.`;
  }

  $("#reply").showModal();
  const text = $("#reply-body");
  text.focus();
  text.setSelectionRange(text.value.length, text.value.length);
}

// Looks the request up, for a chain that knows only the run that asked.
async function openReplyFor(runId) {
  try {
    const { requests } = await get("/help?open=true&window=last_90d");
    const request = requests.find((candidate) => candidate.run_id === runId);
    if (request) openReply(request);
    else alert("That request has already been dealt with.");
  } catch (error) {
    alert(`Could not read it: ${error.message}`);
  }
}

async function submitReply(event) {
  event.preventDefault();
  const body = $("#reply-body").value;
  const error = $("#reply-error");
  if (!body.trim()) {
    error.hidden = false;
    error.textContent = "Say something for the agent to work from.";
    return;
  }

  const by = $("#reply-by").value.trim();
  if (by) localStorage.setItem("layover.replier", by);
  else localStorage.removeItem("layover.replier");

  const payload = { run_id: replying.run_id, body };
  if (by) payload.by = by;
  if (!$("#reply-flags").hidden) {
    payload.flags = {};
    for (const box of $("#reply-flags").querySelectorAll("input[type=checkbox]")) {
      payload.flags[box.dataset.flag] = box.checked;
    }
  }

  $("#reply-send").disabled = true;
  try {
    await send("/help/reply", "POST", payload);
    $("#reply").close();
    loadJournal();
    loadHelpBadge();
    loadChains();
    loadQueued();
  } catch (failure) {
    error.hidden = false;
    error.textContent = `Not sent: ${failure.message}`;
  } finally {
    $("#reply-send").disabled = false;
  }
}

// The trigger dialog, set to a chain's workflow and flags, and saying where they came from.
function continueChain(pipeline, flags, from) {
  openTrigger({ pipeline, flags, from });
}

function startReplies() {
  $("#reply-send").addEventListener("click", submitReply);
}
