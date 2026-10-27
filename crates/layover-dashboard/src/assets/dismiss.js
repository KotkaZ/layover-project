// Dismissing a stalled or failed chain somebody has dealt with another way, and undoing it.
//
// A dismissed chain keeps its state, because that is still what happened, and stops counting as
// needing a person: it leaves "needs attention", the red count on the tab, the count in the title
// and the notifications. A dismissal is of the run the chain ended on, so a chain that goes on and
// ends badly again needs attention again.
//
// Loaded before app.js; uses its helpers ($, el, send, when), chains.js's `ATTENTION` and
// `loadChains`, chain.js's `chainView` and `drawChain`, and attention.js's `attention`, only
// inside functions.

let dismissing = null;

/// Whether a person can dismiss a chain: stalled or failed, and not dismissed yet.
function dismissable(chain) {
  return ["stalled", "failed"].includes(chain.state) && !chain.dismissed;
}

/// Whether a chain needs somebody: what the list's "needs attention" shows.
function needsAttention(chain) {
  return ATTENTION.has(chain.state) && !chain.dismissed;
}

/// "Dismissed by karl · 2 h ago", and why when they said.
function dismissedLine(dismissed) {
  const line = `Dismissed by ${dismissed.by} · ${when(dismissed.at)}`;
  return dismissed.note ? `${line}: ${dismissed.note}` : line;
}

/// A button that opens the dismiss window for `chain`.
function dismissButton(chain) {
  const button = el("button", "link", "Dismiss…");
  button.type = "button";
  button.title = "Dealt with another way? Say so, and it stops needing attention.";
  button.addEventListener("click", () => openDismiss(chain));
  return button;
}

/// A button that undoes `chain`'s dismissal.
function undismissButton(chain) {
  const button = el("button", "link", "Undo dismiss");
  button.type = "button";
  button.title = `${dismissedLine(chain.dismissed)}. Undo it, and it needs attention again.`;
  button.addEventListener("click", () => undoDismiss(chain.itinerary_id));
  return button;
}

function openDismiss(chain) {
  dismissing = chain;
  $("#dismiss-headline").textContent = `Dismiss this ${chain.state} chain`;
  $("#dismiss-meta").textContent = chain.detail ?? "";
  $("#dismiss-note").value = "";
  // The name a reply was last signed with: one person, one name, whichever they did first.
  $("#dismiss-by").value = localStorage.getItem("layover.replier") ?? "";
  $("#dismiss-error").hidden = true;
  $("#dismiss").showModal();
  $("#dismiss-note").focus();
}

async function submitDismiss(event) {
  event.preventDefault();
  const by = $("#dismiss-by").value.trim();
  if (by) localStorage.setItem("layover.replier", by);
  else localStorage.removeItem("layover.replier");

  const payload = {};
  const note = $("#dismiss-note").value.trim();
  if (note) payload.note = note;
  if (by) payload.by = by;

  $("#dismiss-send").disabled = true;
  try {
    await send(`/itineraries/${encodeURIComponent(dismissing.itinerary_id)}/dismiss`, "POST", payload);
    $("#dismiss").close();
    afterDismissal();
  } catch (failure) {
    $("#dismiss-error").hidden = false;
    $("#dismiss-error").textContent = `Not dismissed: ${failure.message}`;
  } finally {
    $("#dismiss-send").disabled = false;
  }
}

async function undoDismiss(id) {
  try {
    await send(`/itineraries/${encodeURIComponent(id)}/dismiss`, "DELETE");
    afterDismissal();
  } catch (failure) {
    alert(`Not undone: ${failure.message}`);
  }
}

/// Everything that counts or shows dismissals, read again — without announcing what the person
/// just did as news: undoing a dismissal raises the count a notification is sent for.
function afterDismissal() {
  attention.known = false;
  loadChains();
  if (chainView.id) drawChain();
}

function startDismissals() {
  $("#dismiss-send").addEventListener("click", submitDismiss);
}
