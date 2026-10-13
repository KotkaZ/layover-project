// Addresses for every view and its filters, so a reload, a bookmark or a link to a colleague
// opens the page where it was: `#runs?status=failed&workflow=eagle-eye`, `#chains?state=stalled`.
//
// A new view is a new history entry, so Back returns to the last one; changing a filter rewrites
// the current entry rather than burying Back under every keystroke. A chain keeps its own form,
// `#chain=<id>`, which links written before this still use.
//
// Uses app.js's `$`, `scope` and `showView`, and chain.js's `chainView`, `openChain` and
// `chainInAddress`.

/// Each view's filters: the address key, and the control it is read from and written to. A
/// control's `data-default` is what it says when the address says nothing.
const ROUTE_PARAMS = {
  chains: {
    window: "#chains-window",
    state: "#chains-state",
    q: "#chains-search",
    routine: "#chains-routine",
  },
  runs: { window: "#runs-window", status: "#runs-status", agent: "#runs-agent" },
  journal: { learnings: "#learn-state", agent: "#learn-agent", q: "#learn-search" },
  upcoming: { hours: "#upcoming-hours" },
};

const router = { applying: false, last: null };

/// Extra address parameters a view keeps outside its controls, by view.
const routeExtras = {};

function currentView() {
  return document.querySelector(".view:not([hidden])")?.id ?? "map";
}

function controlValue(input) {
  if (input.type === "checkbox") return input.checked ? "1" : "";
  return input.value;
}

/// The address the page is at, for `view`.
function routeFor(view) {
  if (view === "chain") {
    return chainView.id ? `#chain=${encodeURIComponent(chainView.id)}` : "#chains";
  }
  const params = new URLSearchParams();
  if (scope()) params.set("workflow", scope());
  for (const [key, selector] of Object.entries(ROUTE_PARAMS[view] ?? {})) {
    const input = $(selector);
    const value = controlValue(input);
    if (value !== (input.dataset.default ?? "")) params.set(key, value);
  }
  for (const [key, value] of Object.entries(routeExtras[view]?.() ?? {})) {
    if (value) params.set(key, value);
  }
  const query = params.toString();
  return `#${view}${query ? `?${query}` : ""}`;
}

/// Records a changed filter in the address, without a new history entry.
function routeChanged() {
  if (router.applying) return;
  const hash = routeFor(currentView());
  router.last = hash;
  if (location.hash !== hash) history.replaceState(null, "", hash);
}

/// Records a change of view as a new history entry.
function routeTo(view) {
  if (router.applying) return;
  const hash = routeFor(view);
  router.last = hash;
  if (location.hash !== hash) history.pushState(null, "", hash);
}

/// Sets a control from the address. A select whose options are not there yet — the agents a
/// learning list has not loaded — keeps the wish until they are.
function setControl(input, value) {
  if (input.type === "checkbox") {
    input.checked = value === "1";
    return;
  }
  if (input.tagName === "SELECT" && value && ![...input.options].some((o) => o.value === value)) {
    input.dataset.wanted = value;
    return;
  }
  input.value = value;
}

/// Puts the page where its address says.
function applyRoute() {
  if (location.hash === router.last) return;
  router.last = location.hash;
  router.applying = true;
  try {
    const chain = chainInAddress();
    if (chain) {
      if (chain !== chainView.id) openChain(chain);
      return;
    }
    const [name, query] = location.hash.replace(/^#/, "").split("?");
    const view = document.getElementById(name)?.classList.contains("view") && name !== "chain"
      ? name
      : "map";
    const params = new URLSearchParams(query ?? "");

    const workflow = params.get("workflow") ?? "";
    if (workflow !== scope() && [...$("#scope").options].some((o) => o.value === workflow)) {
      $("#scope").value = workflow;
      $("#learn-scope").hidden = !workflow;
      // The badges count for the workflow in view; the views themselves load as they are shown.
      loadHelpBadge();
    }
    for (const [key, selector] of Object.entries(ROUTE_PARAMS[view] ?? {})) {
      const input = $(selector);
      setControl(input, params.get(key) ?? input.dataset.default ?? "");
    }
    routeExtras[view]?.(params);
    showView(view);
  } finally {
    router.applying = false;
  }
}

function startRouter() {
  window.addEventListener("popstate", applyRoute);
  window.addEventListener("hashchange", applyRoute);
}
