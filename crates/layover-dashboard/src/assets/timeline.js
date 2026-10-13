// A chain against the clock: each agent's runs as bars on one time axis, and the flags it ran
// with as chips.
//
// The list of runs says what happened in order; it does not show what overlapped, how long a
// review waited for a slot, or that the coder's second pass took three times its first. Bars on
// a shared axis show those at a glance.
//
// Uses app.js's `$`, `el`, `duration`, `pipelinesByName` and `openReport`, and sessions.js's
// `openSession`.

const ENDED = new Set(["succeeded", "failed", "timed_out", "halted", "interrupted"]);

/// The clock time of an instant, to the minute — or to the second when the axis is short.
function axisTime(at, short) {
  return new Date(at).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: short ? "2-digit" : undefined,
    hour12: false,
  });
}

function bar(className, from, to, place, title) {
  const node = el("span", `tl-bar ${className}`);
  const left = place(from);
  node.style.left = `${left}%`;
  node.style.width = `${Math.max(place(to) - left, 0.4)}%`;
  node.title = title;
  return node;
}

/// Draws the chain's runs and waiting flights on one axis, a row per agent in the order they
/// first appeared. Nothing is drawn for a chain with no runs yet: an axis with nothing on it says
/// less than the empty list below it already does.
function chainTimeline(chain, runs, pending) {
  const host = $("#chain-timeline");
  host.replaceChildren();
  host.hidden = runs.length === 0;
  $("#chain-timeline-head").hidden = host.hidden;
  if (host.hidden) return;

  const now = Date.now();
  const startOf = (run) => new Date(run.started_at).getTime();
  const endOf = (run) => (run.finished_at ? new Date(run.finished_at).getTime() : now);
  const start = Math.min(new Date(chain.started_at).getTime(), ...runs.map(startOf));
  const live = runs.some((run) => !ENDED.has(run.status)) || pending.length > 0;
  const end = live ? now : Math.max(...runs.map(endOf));
  const span = Math.max(end - start, 1000);
  const place = (at) => Math.min(100, Math.max(0, ((at - start) / span) * 100));

  const agents = [];
  for (const name of [...runs.map((run) => run.agent), ...pending.map((flight) => flight.to)]) {
    if (!agents.includes(name)) agents.push(name);
  }

  for (const agent of agents) {
    const row = el("div", "tl-row");
    const track = el("div", "tl-track");
    for (const run of runs.filter((r) => r.agent === agent)) {
      const took = duration(Math.round((endOf(run) - startOf(run)) / 1000));
      const status = run.status.replace("_", " ");
      const node = bar(
        run.status,
        startOf(run),
        endOf(run),
        place,
        `${agent} · ${status} · ${took}\nstarted ${new Date(run.started_at).toLocaleString()}` +
          `${run.detail ? `\n${run.detail}` : ""}\n${run.status === "running" ? "Click to watch it" : "Click for its report"}`,
      );
      node.addEventListener("click", () =>
        run.status === "running" ? openSession(run) : openReport(run.run_id, run),
      );
      track.append(node);
    }
    // Work waiting for a slot, from when it was queued until now.
    for (const flight of pending.filter((f) => f.to === agent)) {
      const queued = new Date(flight.queued_at).getTime();
      track.append(
        bar("queued", queued, now, place, `${agent} · queued ${duration(Math.round((now - queued) / 1000))} ago, waiting for a slot`),
      );
    }
    const name = el("span", "tl-agent", agent);
    name.title = agent;
    row.append(name, track);
    host.append(row);
  }

  // Five marks along the axis: the start as a clock time, the rest as time since it.
  const axis = el("div", "tl-row tl-axis");
  const marks = el("div", "tl-track");
  const short = span < 10 * 60 * 1000;
  for (const share of [0, 0.25, 0.5, 0.75, 1]) {
    const label = share === 0
      ? axisTime(start, short)
      : `+${duration(Math.round((span * share) / 1000))}`;
    const mark = el("span", "tl-mark", label);
    mark.style.left = `${share * 100}%`;
    marks.append(mark);
  }
  axis.append(el("span", "tl-agent", live ? "until now" : ""), marks);
  host.append(axis);
}

/// The flags a chain ran with, as chips: those on, and any off that its workflow would have had
/// on. A flag that differs from the workflow's default is marked, since that is what somebody
/// chose when they triggered it. The rest — off, as they would have been — are counted rather than
/// listed, because a workflow with twenty flags would otherwise bury the two that are on.
function chainFlags(chain) {
  const host = $("#chain-flags");
  host.replaceChildren();
  const flags = Object.entries(chain.flags ?? {}).sort(([a], [b]) => a.localeCompare(b));
  host.hidden = flags.length === 0;
  if (host.hidden) return;

  const defaults = new Map(
    (pipelinesByName.get(chain.pipeline)?.flags ?? []).map((flag) => [flag.name, flag]),
  );
  host.append(el("span", "muted", "Flags"));
  const quiet = [];
  for (const [name, on] of flags) {
    const declared = defaults.get(name);
    const chosen = declared !== undefined && declared.default !== on;
    if (!on && !chosen) {
      quiet.push(name);
      continue;
    }
    const chip = el("span", `flag-chip ${on ? "on" : "off"}${chosen ? " chosen" : ""}`, name);
    chip.title = [
      `${name} was ${on ? "on" : "off"} in this chain`,
      chosen ? `not the default: ${chain.pipeline} has it ${declared.default ? "on" : "off"}` : "the default",
      declared?.description,
    ]
      .filter(Boolean)
      .join("\n");
    host.append(chip);
  }
  if (quiet.length) {
    const rest = el("span", "muted", `${quiet.length} off, as by default`);
    rest.title = quiet.join(", ");
    host.append(rest);
  }
}
