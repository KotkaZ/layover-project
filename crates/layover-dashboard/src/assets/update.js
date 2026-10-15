// The other half of the version pill: a newer Layover, and installing it.
//
// Read from `GET /update`, which says what the machine last learned and never asks GitHub itself,
// so loading it costs nothing. The Tower looks once a day; this reads what it found every ten
// minutes, and every few seconds while an install is under way.

let updateStatus = null;
let updateTimer = null;

async function loadUpdate() {
  try {
    updateStatus = await get("/update");
  } catch {
    updateStatus = null;
  }
  paintBanner();
  // The pill is drawn in one place, which asks this file what to add.
  loadHealth();
  // Watched closely while the Tower installs, and at the usual pace otherwise.
  clearTimeout(updateTimer);
  updateTimer = setTimeout(loadUpdate, underWay() ? 3_000 : 600_000);
}

/// The install or rollback under way, if one is.
function underWay() {
  const request = updateStatus?.request;
  return request && !request.outcome ? request : null;
}

/// What the pill adds after the version: " → 1.15.0" when one is available, " · updating" while
/// one is being installed.
function updateSuffix() {
  if (underWay()) return " · updating";
  const found = updateStatus?.available;
  return found ? ` → ${found.version}` : "";
}

/// Whether the update window has anything to say.
function updateWorthOpening() {
  return Boolean(updateStatus?.available || updateStatus?.previous || underWay());
}

/// Marks the pill as opening the update window, or as only a version. Called by `loadHealth`
/// once it has written the pill's text, so a Ground Stop or a Tower stopping keeps its colour.
function paintUpdate() {
  const pill = $("#health");
  const found = updateStatus?.available;
  const plain = !pill.classList.contains("down") && !pill.classList.contains("stopping");
  pill.classList.toggle("update", Boolean(found) && plain && !underWay());
  if (underWay() && plain) pill.classList.add("stopping");
  if (updateWorthOpening()) {
    pill.setAttribute("role", "button");
    pill.tabIndex = 0;
    if (plain && found) {
      pill.title = `Layover ${found.version} is available (${found.kind}). Open it to see what changes.`;
    }
  } else {
    pill.removeAttribute("role");
    pill.removeAttribute("tabindex");
  }
}

/// The line above every view while the Tower installs a release.
function paintBanner() {
  const banner = $("#update-banner");
  const request = underWay();
  banner.hidden = !request;
  if (!request) return;
  const last = request.progress.at(-1) ?? "Asked; the Tower will drain for it.";
  $("#update-banner-text").textContent =
    `${request.rollback ? "Going back to" : "Installing"} ${request.version} — ${last}`;
}

/// Opens what the newer release changes, and what can be done about it.
function openUpdate() {
  if (!updateWorthOpening()) return;
  const found = updateStatus.available;
  const staged = updateStatus.staged;
  const gate = updateStatus.gate;
  if (found) {
    const released = found.published_at ? ` · released ${found.published_at.slice(0, 10)}` : "";
    const checked = updateStatus.succeeded_at
      ? ` · checked ${checkedAgo(updateStatus.succeeded_at)}`
      : "";
    $("#update-headline").textContent = `Layover ${found.version} is available`;
    $("#update-meta").textContent =
      `${found.kind}${found.prerelease ? " pre-release" : ""}${released} · this is ${updateStatus.current}${checked}`;
    markdownish(
      $("#update-compatibility"),
      found.compatibility ?? "The release notes do not say what an existing factory will notice.",
    );
    markdownish($("#update-notes"), found.notes);
    $("#update-page").href = found.url;
    $("#update-skip").textContent = `layover update skip ${found.version}`;
  } else {
    $("#update-headline").textContent = `Layover ${updateStatus.current}`;
    $("#update-meta").textContent = lastUpdateSays(updateStatus.last_update);
    $("#update-compatibility").textContent = "";
    $("#update-notes").textContent = "";
  }
  for (const id of ["#update-compatibility", "#update-details", "#update-how", "#update-page"]) {
    $(id).hidden = !found;
  }
  $("#update-staged").textContent = stagedSays(staged, gate);
  $("#update-staged").hidden = !staged || !found;

  const installable = found && staged?.ready && gate?.passes && !staged.crosses_layout;
  $("#update-apply").hidden = !installable || Boolean(underWay());
  $("#update-apply").textContent = `Update to ${found?.version ?? ""} now`;
  $("#update-skip-button").hidden = !found;
  $("#update-rollback").hidden = !updateStatus.previous || Boolean(underWay());
  $("#update-rollback").textContent = `Roll back to ${updateStatus.previous ?? ""}`;
  $("#update-dialog").showModal();
}

/// One line about the last update: when, and whether it proved itself.
function lastUpdateSays(last) {
  if (!last) return "";
  const when = last.started.slice(0, 16).replace("T", " ");
  const how = last.rolled_back_at
    ? "rolled back"
    : last.healthy_at
      ? "healthy"
      : "not yet proved healthy";
  return `Updated from ${last.from} to ${last.to} on ${when} — ${how}.`;
}

/// Asks the Tower to install, roll back, skip or stop installing, and says so if it refuses.
async function askUpdate(method, path, body) {
  try {
    const response = await fetch(path, {
      method,
      headers: { accept: "application/json", "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!response.ok) {
      const problem = await response.json().catch(() => ({}));
      throw new Error(problem.detail ?? problem.title ?? `the Tower answered ${response.status}`);
    }
    updateStatus = await response.json();
  } catch (error) {
    alert(`Could not do that: ${error.message}`);
  }
  $("#update-dialog").close();
  loadUpdate();
}

function applyUpdate() {
  const found = updateStatus?.available;
  if (!found) return;
  const confirmed = confirm(
    `Install ${found.version}? Nothing new starts while what is running finishes, then the Tower ` +
      "restarts on the new version.",
  );
  if (confirmed) askUpdate("POST", "/update/apply", {});
}

function rollBackUpdate() {
  const previous = updateStatus?.previous;
  if (previous && confirm(`Go back to ${previous}? ${updateStatus.current} will not be offered again.`)) {
    askUpdate("POST", "/update/rollback");
  }
}

function skipUpdate() {
  const found = updateStatus?.available;
  if (found) askUpdate("POST", "/update/skip", { version: found.version });
}

/// What staging made of the release, and what it made of this factory, in a sentence or two.
function stagedSays(staged, gate) {
  if (!staged) return "";
  if (staged.refused) return `Staging it was refused: ${staged.refused.split("\n")[0]}`;
  const parts = [];
  if (staged.sha256) parts.push("Downloaded; its SHA-256 matches the release.");
  if (staged.provenance === "verified") parts.push(`Built by ${staged.provenance_detail}.`);
  else if (staged.provenance === "unavailable")
    parts.push(`Its provenance could not be checked (${staged.provenance_detail}), so it was not run.`);
  if (staged.crosses_layout) parts.push("It reads a different state layout: installing it is your choice.");
  if (gate?.not_run) parts.push(`Not checked against this factory: ${gate.not_run}`);
  else if (gate?.errors.length) parts.push(`It refuses this factory: ${gate.errors.join("; ")}`);
  else if (gate?.new_warnings.length)
    parts.push(`It validates this factory, with warnings this version does not give: ${gate.new_warnings.join("; ")}`);
  else if (gate) parts.push("It validates this factory, with no new warnings.");
  return parts.join(" ");
}

/// Release notes as written — Markdown, wrapped at a hundred columns — with their bold, emphasis
/// and code drawn as such. Built from text nodes, so nothing in the notes is ever read as HTML.
function markdownish(target, text) {
  target.replaceChildren();
  marked(target, text);
}

function marked(target, text) {
  const marks = /(\*\*[^*]+\*\*|`[^`]+`|\*[^*\s][^*]*\*)/g;
  let at = 0;
  for (const match of text.matchAll(marks)) {
    target.append(text.slice(at, match.index));
    const token = match[0];
    if (token.startsWith("`")) {
      target.append(el("code", "", token.slice(1, -1)));
    } else {
      const bold = token.startsWith("**");
      const node = el(bold ? "b" : "em");
      // Code inside bold is common in these notes: `**`layover run` waits**`.
      marked(node, bold ? token.slice(2, -2) : token.slice(1, -1));
      target.append(node);
    }
    at = match.index + token.length;
  }
  target.append(text.slice(at));
}

/// "3 hours ago", for when a check last worked.
function checkedAgo(at) {
  const hours = Math.max(0, Math.round((Date.now() - Date.parse(at)) / 3_600_000));
  if (hours < 1) return "within the hour";
  if (hours < 48) return `${hours} hour${hours === 1 ? "" : "s"} ago`;
  return `${Math.round(hours / 24)} days ago`;
}

function startUpdate() {
  const pill = $("#health");
  pill.addEventListener("click", openUpdate);
  pill.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      openUpdate();
    }
  });
  $("#update-apply").addEventListener("click", applyUpdate);
  $("#update-skip-button").addEventListener("click", skipUpdate);
  $("#update-rollback").addEventListener("click", rollBackUpdate);
  $("#update-cancel").addEventListener("click", () => askUpdate("DELETE", "/update/apply"));
  loadUpdate();
}
