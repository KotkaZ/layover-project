// The other half of the version pill: a newer Layover, when this machine has found one.
//
// Read from `GET /update`, which says what the machine last learned and never asks GitHub itself,
// so loading it costs nothing. The Tower looks once a day; this reads what it found every ten
// minutes.

let updateStatus = null;

async function loadUpdate() {
  try {
    updateStatus = await get("/update");
  } catch {
    updateStatus = null;
  }
  // The pill is drawn in one place, which asks this file what to add.
  loadHealth();
}

/// What the pill adds after the version: " → 1.15.0" when one is available.
function updateSuffix() {
  const found = updateStatus?.available;
  return found ? ` → ${found.version}` : "";
}

/// Marks the pill as opening the release's notes, or as only a version. Called by `loadHealth`
/// once it has written the pill's text, so a Ground Stop or a Tower stopping keeps its colour.
function paintUpdate() {
  const pill = $("#health");
  const found = updateStatus?.available;
  const plain = !pill.classList.contains("down") && !pill.classList.contains("stopping");
  pill.classList.toggle("update", Boolean(found) && plain);
  if (found) {
    pill.setAttribute("role", "button");
    pill.tabIndex = 0;
    if (plain) {
      pill.title = `Layover ${found.version} is available (${found.kind}). Open it to see what changes.`;
    }
  } else {
    pill.removeAttribute("role");
    pill.removeAttribute("tabindex");
  }
}

/// Opens what the newer release says it changes.
function openUpdate() {
  const found = updateStatus?.available;
  if (!found) return;
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
  $("#update-dialog").showModal();
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
  loadUpdate();
  setInterval(loadUpdate, 600_000);
}
