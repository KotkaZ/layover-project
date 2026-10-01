// Watching agents work: every running session, and the replay of ones that are over.
//
// Read-only by construction. Each terminal is an EventSource on `/runs/{id}/stream`, which sends
// what the agent's CLI printed, already rendered the way the CLI would draw it in a terminal —
// there is nothing here to type into, and nothing an agent can tell from being watched.
//
// Loaded before app.js, so it uses the helpers there ($, el, get, when, duration, scoped) only
// inside functions that run after both have loaded.

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
// More tiles than this would hold enough connections open to starve the rest of the page.
const MAX_TILES = 4;

const sessions = { terminals: new Map(), tiled: false, timer: null };

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
    head.append(this.dot, this.title, this.state, el("span", "ro", "read-only"), close);

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
    const parts = [run.agent, run.pipeline, run.model].filter(Boolean);
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

function forget(terminal) {
  terminal.close();
  sessions.terminals.delete(terminal.run.run_id);
  markOpen();
}

// Shows one session, on its own.
function openSession(run) {
  sessions.tiled = false;
  $("#sessions-tile").setAttribute("aria-pressed", "false");
  for (const terminal of sessions.terminals.values()) terminal.close();
  sessions.terminals.clear();
  watch(run);
  showView("sessions");
}

function watch(run) {
  if (sessions.terminals.has(run.run_id)) return;
  $("#sessions-empty").hidden = true;
  sessions.terminals.set(run.run_id, new Terminal(run, $("#terminals"), forget));
  markOpen();
}

// Every session that is running, side by side, as many as fit.
function tile(running) {
  for (const run of running) {
    if (sessions.terminals.has(run.run_id)) continue;
    if (sessions.terminals.size >= MAX_TILES) {
      // Make room by letting go of a session that has ended, never of one still going.
      const over = [...sessions.terminals.values()].find((terminal) => terminal.ended);
      if (!over) break;
      forget(over);
    }
    watch(run);
  }
}

function markOpen() {
  document.querySelectorAll("#session-list .session").forEach((item) => {
    item.classList.toggle("open", sessions.terminals.has(item.dataset.run));
  });
  $("#terminals").classList.toggle("tiled", sessions.terminals.size > 1);
  $("#terminals").classList.toggle("crowded", sessions.terminals.size > 2);
  $("#sessions-empty").hidden = sessions.terminals.size > 0;
}

function sessionItem(run) {
  const item = el("button", "session");
  item.type = "button";
  item.dataset.run = run.run_id;
  const top = el("span", "who");
  top.append(el("span", `dot ${run.status}`), el("b", "", run.agent), el("span", "muted", run.pipeline ?? ""));
  const sub = el(
    "span",
    "muted",
    run.status === "running"
      ? `running ${duration(run.duration_sec)}`
      : `${run.status.replace("_", " ")} ${when(run.finished_at)}`,
  );
  item.append(top, sub);
  item.title = `${run.run_id}${run.detail ? `\n${run.detail}` : ""}`;
  item.addEventListener("click", () => (sessions.tiled ? watch(run) : openSession(run)));
  return item;
}

async function loadSessions() {
  const list = $("#session-list");
  try {
    const params = scoped(new URLSearchParams({ window: "last_24h", limit: "40" }));
    const { runs } = await get(`/runs?${params}`);
    const running = runs.filter((run) => run.status === "running");
    const over = runs.filter((run) => run.status !== "running").slice(0, 25);

    list.replaceChildren(el("h2", "", `Running now · ${running.length}`));
    if (running.length === 0) list.append(el("p", "hint", "Nothing is running."));
    running.forEach((run) => list.append(sessionItem(run)));
    list.append(el("h2", "", "Ended in the last 24 hours"));
    if (over.length === 0) list.append(el("p", "hint", "Nothing has ended."));
    over.forEach((run) => list.append(sessionItem(run)));

    if (sessions.tiled) tile(running);
    markOpen();
    setLiveBadge(running.length);
  } catch (error) {
    list.replaceChildren(el("p", "empty", `Could not read the sessions: ${error.message}`));
  }
}

function setLiveBadge(count) {
  const badge = $("#live-badge");
  badge.textContent = String(count);
  badge.hidden = count === 0;
}

// Kept current only while it is being looked at.
function watchSessions(visible) {
  clearInterval(sessions.timer);
  sessions.timer = null;
  if (!visible) return;
  loadSessions();
  sessions.timer = setInterval(loadSessions, 5000);
}

function startSessions() {
  $("#sessions-tile").addEventListener("click", () => {
    sessions.tiled = !sessions.tiled;
    $("#sessions-tile").setAttribute("aria-pressed", String(sessions.tiled));
    loadSessions();
  });
  $("#sessions-thinking").addEventListener("change", (event) => {
    $("#terminals").classList.toggle("hide-think", !event.target.checked);
  });
}
