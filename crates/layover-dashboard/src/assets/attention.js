// What needs a person, said where a person will see it: in the tab's title, and — if they ask for
// it — as a notification from the browser.
//
// A factory runs unattended, so the page is usually in a background tab when an agent stops to
// ask something. The badges on the tabs only help somebody already looking at the page; the
// title is visible from the tab strip, and a notification from anywhere.
//
// Uses app.js's `$`, `loadHelpBadge` and `currentView` (router.js), and chains.js's `loadChains`,
// which report their counts here.

const NOTIFY = "layover.notify";
const ATTENTION_EVERY_MS = 30000;
// `known` is false until both counts have been read once, so opening the page does not announce
// everything already waiting as though it had just happened.
const attention = {
  help: null,
  helpScope: "",
  stalled: null,
  failed: null,
  awaiting: null,
  known: false,
  timer: null,
};

function notifying() {
  try {
    return localStorage.getItem(NOTIFY) === "on" && Notification.permission === "granted";
  } catch {
    return false;
  }
}

function canNotify() {
  return "Notification" in window && window.isSecureContext;
}

/// Says, in a notification, what has newly come to need somebody.
function announce(before) {
  const news = [];
  const more = (now, was) => Math.max(0, (now ?? 0) - (was ?? 0));
  const waiting = more(attention.awaiting, before.awaiting);
  const stalled = more(attention.stalled, before.stalled);
  const failed = more(attention.failed, before.failed);
  // Narrowing the page to another workflow changes the help count without anybody asking anything.
  const help = attention.helpScope === before.helpScope ? more(attention.help, before.help) : 0;
  if (waiting) news.push(`${waiting} chain(s) waiting for you`);
  if (stalled) news.push(`${stalled} chain(s) stalled`);
  if (failed) news.push(`${failed} chain(s) ended on a failed run`);
  if (help) news.push(`${help} new help request(s)`);
  if (news.length === 0 || !notifying()) return;

  const note = new Notification("Layover needs you", {
    body: news.join(" · "),
    tag: "layover-attention",
  });
  note.addEventListener("click", () => {
    window.focus();
    // Straight to what it is about: a stopped chain on Chains, a question on Help.
    if (waiting || stalled || failed) {
      $("#chains-state").value = "attention";
      showView("chains");
    } else {
      showView("journal");
    }
    note.close();
  });
}

/// Takes a fresh count from whoever read one, and says it in the title.
function noteAttention(counts) {
  const before = { ...attention };
  Object.assign(attention, counts);
  if (attention.known) announce(before);
  attention.known = attention.help !== null && attention.stalled !== null;

  // Open help requests, and chains nothing will move again by itself: stalled, or ended on a run
  // that failed. A chain waiting for you is already counted: what it waits on is an open help
  // request.
  const count = (attention.help ?? 0) + (attention.stalled ?? 0) + (attention.failed ?? 0);
  document.title = count > 0 ? `(${count}) Layover` : "Layover";
}

function showNotifySwitch() {
  const button = $("#notify");
  button.hidden = !canNotify();
  if (button.hidden) return;
  const on = notifying();
  button.setAttribute("aria-pressed", String(on));
  button.title = on
    ? "Notifying you when a chain waits for you, stalls or fails, an agent asks for help, or Layover has a new release, updates or rolls back. Click to stop."
    : Notification.permission === "denied"
      ? "Notifications are blocked for this page in the browser's settings."
      : "Notify me when a chain waits for me, stalls or fails, an agent asks for help, or Layover has a new release, updates or rolls back.";
}

async function toggleNotify() {
  if (notifying()) {
    localStorage.setItem(NOTIFY, "off");
  } else {
    const permission = Notification.permission === "default"
      ? await Notification.requestPermission()
      : Notification.permission;
    if (permission === "granted") localStorage.setItem(NOTIFY, "on");
  }
  showNotifySwitch();
}

function startAttention() {
  showNotifySwitch();
  $("#notify").addEventListener("click", toggleNotify);
  // The counts behind the badges and the title are kept current wherever the page is; the Chains
  // tab keeps its own, faster, count while it is showing.
  attention.timer = setInterval(() => {
    loadHelpBadge();
    if (currentView() !== "chains") loadChains();
  }, ATTENTION_EVERY_MS);
}
