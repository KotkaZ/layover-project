// Tracing one agent's routes on a busy route map.
//
// The map arrives already drawn, and every route in it names the two boxes it joins
// (`data-from`, `data-to`). This lights an agent's routes and neighbours while it is hovered or
// focused, and fades everything else; a click pins it and opens a panel saying what the agent runs
// on and who it talks to *in this workflow*. Nothing here knows about layout — it only reads the
// drawing it is handed — so the geometry stays in Rust, where it is tested.
//
// Separate from app.js because the page script is already long, and this is one self-contained
// behaviour of one element.

/// Wires one drawn map.
///
/// `describe(name)` returns the agent as `GET /agents` lists it, or undefined; `showRuns(name)`
/// opens the run history for one agent.
function traceable(canvas, { describe, showRuns }) {
  const svg = canvas.querySelector("svg.routemap");
  if (!svg) return;

  // Its own, rather than app.js's `el`: this file loads first and stands on its own.
  const make = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };

  const links = [...svg.querySelectorAll(".route")];
  const nodes = [...svg.querySelectorAll(".node")];
  const panel = document.createElement("div");
  panel.className = "trace";
  panel.hidden = true;
  canvas.after(panel);
  let pinned = null;

  // Lights one node, every route touching it, and the node at the other end of each.
  function light(id) {
    svg.classList.toggle("tracing", id !== null);
    const near = new Set(id === null ? [] : [id]);
    for (const link of links) {
      const on = id !== null && (link.dataset.from === id || link.dataset.to === id);
      link.classList.toggle("lit", on);
      if (on) {
        near.add(link.dataset.from);
        near.add(link.dataset.to);
      }
    }
    for (const node of nodes) node.classList.toggle("lit", near.has(node.id));
  }

  // Lights one route and its two ends.
  function lightLink(link) {
    svg.classList.add("tracing");
    for (const other of links) other.classList.toggle("lit", other === link);
    for (const node of nodes) {
      node.classList.toggle("lit", node.id === link.dataset.from || node.id === link.dataset.to);
    }
  }

  // Back to whatever is pinned, or to nothing, when the pointer or focus moves on.
  const rest = () => light(pinned);

  function pin(id) {
    pinned = id;
    for (const node of nodes) node.classList.toggle("pinned", node.id === id);
    light(id);
    show(id);
  }

  for (const node of nodes) {
    node.addEventListener("mouseenter", () => light(node.id));
    node.addEventListener("mouseleave", rest);
    node.addEventListener("focus", () => light(node.id));
    node.addEventListener("blur", rest);
    node.addEventListener("click", (event) => {
      event.stopPropagation();
      pin(pinned === node.id ? null : node.id);
    });
    node.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        pin(pinned === node.id ? null : node.id);
      }
      if (event.key === "Escape") pin(null);
    });
  }
  for (const link of links) {
    link.addEventListener("mouseenter", () => lightLink(link));
    link.addEventListener("mouseleave", rest);
  }
  // A click on empty space lets go.
  svg.addEventListener("click", () => pin(null));

  function show(id) {
    if (id === null) {
      panel.hidden = true;
      panel.replaceChildren();
      return;
    }

    const name = (nodeId) => nodeId.replace(/^[ap]_/, "");
    const sends = new Set();
    const hears = new Set();
    for (const link of links) {
      const { from, to } = link.dataset;
      const both = link.classList.contains("both");
      if (from === id) {
        sends.add(to);
        if (both) hears.add(to);
      }
      if (to === id) {
        hears.add(from);
        if (both) sends.add(from);
      }
    }
    const list = (ids) =>
      ids.size === 0
        ? "nobody in this workflow"
        : [...ids]
            .map((other) => (other.startsWith("p_") ? `${name(other)} (way in)` : name(other)))
            .sort()
            .join(", ");

    const title = make("header");
    title.append(make("b", "", name(id)));
    const agent = id.startsWith("a_") ? describe(name(id)) : undefined;
    if (agent?.description) title.append(make("span", "what", agent.description));

    const actions = make("span", "actions");
    if (id.startsWith("a_")) {
      const runs = make("button", "link", "Its runs");
      runs.type = "button";
      runs.addEventListener("click", () => showRuns(name(id)));
      actions.append(runs);
    }
    const close = make("button", "link", "Close");
    close.type = "button";
    close.addEventListener("click", () => pin(null));
    actions.append(close);
    title.append(actions);

    const facts = make("dl");
    const fact = (label, value) => facts.append(make("dt", "", label), make("dd", "", value));
    if (id.startsWith("a_")) {
      // Null means the command line does not say, so the CLI decides — which is worth saying
      // rather than leaving blank, because blank reads as "Layover does not know".
      const fromCli = "the CLI's own default";
      fact("Model", agent?.model ?? fromCli);
      fact("Effort", agent?.reasoning_effort ?? fromCli);
      fact("Context", agent?.context ?? fromCli);
      if (agent) {
        fact("Runner", `${agent.runner ?? "the default runner"} · ${agent.access}`);
      }
    }
    fact("Sends to", list(sends));
    fact("Hears from", list(hears));

    panel.replaceChildren(title, facts);
    panel.hidden = false;
  }
}
