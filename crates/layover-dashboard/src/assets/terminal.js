// One session's terminal: what an agent's CLI printed, drawn the way the CLI would draw it.
//
// Read-only by construction. Each terminal is an EventSource on `/runs/{id}/stream`, which sends
// what the agent's CLI printed, already rendered the way the CLI would draw it in a terminal —
// there is nothing here to type into, and nothing an agent can tell from being watched.
//
// Loaded before app.js, so it uses the helpers there ($, el, openReport) and chain.js's openChain
// only inside methods that run after all have loaded.

// The prefix each kind of entry is drawn with, as the CLI itself marks them.
const MARKS = {
  prompt: "›",
  think: "∴",
  say: "",
  tool: "●",
  done: "└",
  failed: "✗",
  info: "·",
  raw: "",
};

// Far more than anyone scrolls back through, and few enough that a page tiling four sessions for
// an afternoon stays quick.
const KEEP_LINES = 4000;

class Terminal {
  constructor(run, host, onClose) {
    this.run = run;
    this.last = null;
    this.ended = false;
    this.trimmed = false;

    this.node = el("div", "terminal");
    const head = el("div", "term-head");
    this.dot = el("span", "dot running");
    this.title = el("span", "term-title");
    this.state = el("span", "term-state", "connecting…");
    const close = el("button", "term-close", "×");
    close.title = "Stop watching";
    close.addEventListener("click", () => onClose(this));
    // Where the session belongs, and what it wrote about itself once it is over.
    const chain = el("button", "term-link", "chain");
    chain.type = "button";
    chain.title = `See chain ${run.itinerary_id} whole`;
    chain.addEventListener("click", () => openChain(run.itinerary_id));
    this.report = el("button", "term-link", "report");
    this.report.type = "button";
    this.report.title = "What the agent wrote about this run";
    this.report.hidden = run.status === "running";
    this.report.addEventListener("click", () => openReport(run.run_id, run));
    head.append(this.dot, this.title, this.state, chain, this.report, el("span", "ro", "read-only"), close);

    this.body = el("div", "term-body");
    this.lines = el("div", "term-lines");
    this.live = el("div", "term-live");
    this.body.append(this.lines, this.live);
    this.node.append(head, this.body);
    host.append(this.node);

    this.describe();
    this.connect();
  }

  describe() {
    const run = this.run;
    const effort = run.reasoning_effort ? `effort ${run.reasoning_effort}` : null;
    const context = run.context ? run.context.replace(/[_-]/g, " ") : null;
    const parts = [run.agent, run.chain_name, run.pipeline, run.model, effort, context].filter(Boolean);
    this.title.textContent = parts.join(" · ");
    this.title.title = `${run.run_id}\nchain ${run.itinerary_id}`;
  }

  connect() {
    const after = this.last ? `?after=${encodeURIComponent(this.last)}` : "";
    const source = new EventSource(`/runs/${encodeURIComponent(this.run.run_id)}/stream${after}`);
    this.source = source;

    source.addEventListener("open", () => {
      // Whatever was half-typed when the connection dropped will not be finished by this one.
      this.live.replaceChildren();
      this.state.textContent = this.run.status === "running" ? "live" : "replay";
    });
    source.addEventListener("entry", (event) => {
      this.last = event.lastEventId;
      this.entry(JSON.parse(event.data));
    });
    source.addEventListener("partial", (event) => {
      this.last = event.lastEventId;
      this.partial(JSON.parse(event.data));
    });
    source.addEventListener("end", (event) => {
      source.close();
      this.end(JSON.parse(event.data));
    });
    // The browser would reconnect by itself, from the start. Doing it by hand resumes where the
    // stream broke off instead of replaying the whole session again.
    source.onerror = () => {
      source.close();
      if (this.ended || this.closed) return;
      this.state.textContent = "reconnecting…";
      setTimeout(() => {
        if (!this.closed) this.connect();
      }, 2000);
    };
  }

  close() {
    this.closed = true;
    this.source?.close();
    this.node.remove();
  }

  following() {
    if ($("#sessions-follow").checked) return true;
    const body = this.body;
    return body.scrollHeight - body.scrollTop - body.clientHeight < 40;
  }

  line(kind, text, at) {
    const row = el("div", `line ${kind}`);
    row.append(
      el("span", "t", at ? new Date(at).toLocaleTimeString([], { hour12: false }) : ""),
      el("span", "m", MARKS[kind] ?? ""),
      el("span", "x", text),
    );
    return row;
  }

  entry(entry) {
    const follow = this.following();
    if (entry.closes) this.live.querySelector(`[data-id="${CSS.escape(entry.closes)}"]`)?.remove();
    this.lines.append(this.line(entry.kind, entry.text, entry.at));

    if (this.lines.childElementCount > KEEP_LINES) {
      for (let i = 0; i < KEEP_LINES / 10; i++) this.lines.firstElementChild?.remove();
      if (!this.trimmed) {
        this.trimmed = true;
        this.lines.prepend(this.line("info", "earlier output is not kept on the page; reopen the session to see it all"));
      }
    }
    if (follow) this.body.scrollTop = this.body.scrollHeight;
  }

  partial(partial) {
    const follow = this.following();
    let row = this.live.querySelector(`[data-id="${CSS.escape(partial.id)}"]`);
    if (!partial.text) {
      row?.remove();
      return;
    }
    if (!row) {
      row = this.line(partial.kind, "");
      row.classList.add("typing");
      row.dataset.id = partial.id;
      this.live.append(row);
    }
    row.querySelector(".x").textContent = partial.text;
    if (follow) this.body.scrollTop = this.body.scrollHeight;
  }

  end(end) {
    this.ended = true;
    this.report.hidden = false;
    this.live.replaceChildren();
    const status = end.status ?? "over";
    this.dot.className = `dot ${status}`;
    this.state.textContent = status.replace("_", " ");
    if (end.missing) {
      this.lines.append(
        this.line("info", "No transcript was kept for this run: it is past the 90-day horizon, or never started."),
      );
    }
    const closing = el("div", `line end ${status}`, `── ${status.replace("_", " ")} ──`);
    if (end.detail) closing.title = end.detail;
    this.lines.append(closing);
    if (this.following()) this.body.scrollTop = this.body.scrollHeight;
  }
}
