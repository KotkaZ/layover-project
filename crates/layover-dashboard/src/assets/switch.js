// Switching a workflow off and on: one switch per workflow, beside its name on the route map.
//
// Off closes the workflow's ways in — its schedule does not fire, a trigger is refused, a resuming
// workflow collects nothing, a trigger already queued waits — and lets every chain of it already
// under way finish. Switching it back on needs no confirmation, unlike releasing a Ground Stop: it
// starts nothing at once except a trigger it held, and its schedule fires at its next tick.
//
// Separate from app.js, which is already long; it uses that file's helpers (`send`, `el`, `$`,
// `pipelinesByName`, `loadPipelines`, `loadMap`, `loadQueued`) and upcoming.js's `clockTime`.

/// The switch for one workflow, as a labelled checkbox: checked is on.
function workflowSwitch(pipeline) {
  const label = el("label", `switch${pipeline.switched_off ? " off" : ""}`);
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = !pipeline.switched_off;
  box.dataset.workflow = pipeline.name;
  const text = el("span", "", pipeline.switched_off ? "Off" : "On");
  label.title = pipeline.switched_off
    ? `${pipeline.name} is switched off${pipeline.switched_off_since ? ` since ${new Date(pipeline.switched_off_since).toLocaleString()}` : ""}: ` +
      "its schedule does not fire and it cannot be triggered. Chains already under way finish. Click to switch it on."
    : `Switch ${pipeline.name} off: its schedule stops firing and it cannot be triggered until switched back on. Chains already under way finish.`;
  box.addEventListener("change", () => setWorkflow(pipeline.name, box.checked, box));
  label.append(box, text);
  return label;
}

/// A rail saying a workflow is off, in place of when it next fires.
function offRail(pipeline) {
  const since = pipeline.switched_off_since;
  return rail("off", "", since ? `switched off · since ${clockTime(since)}` : "switched off");
}

async function setWorkflow(name, on, box) {
  box.disabled = true;
  try {
    await send(`/pipelines/${encodeURIComponent(name)}/switch-off`, on ? "DELETE" : "POST");
  } catch (error) {
    alert(`Could not switch ${name} ${on ? "on" : "off"}: ${error.message}`);
  }
  await loadPipelines();
  loadMap(true);
  loadQueued();
  if (!$("#upcoming").hidden) loadUpcoming();
}

/// In the trigger window: a workflow that is off cannot be queued, and says so before anybody
/// writes a prompt for it.
function showSwitch(name) {
  const off = pipelinesByName.get(name)?.switched_off ?? false;
  const note = $("#trigger-off");
  note.hidden = !off;
  note.textContent = off
    ? `${name} is switched off, so it cannot be triggered. Switch it on beside its name on the route map first.`
    : "";
  $("#trigger-send").disabled = off;
}
