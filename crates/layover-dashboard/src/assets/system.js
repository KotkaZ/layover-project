// System: the machine the factory runs on — how busy its processors are, how much memory and disk
// space it has left — and how much of each Layover's own processes use.
//
// Drawn here as SVG from `GET /system`, with no charting library: three line graphs are a few
// dozen lines, and a library would be the largest file the page loads. The machine is not any one
// workflow's, so the Workflow selector narrows nothing here.
//
// Uses app.js's `get`, `el`, `$` and `updated`, and router.js's `routeChanged`, inside functions.

const SVG_NS = "http://www.w3.org/2000/svg";
// Each graph is drawn in this box and stretched to its card's width.
const GRAPH = { w: 600, h: 150 };
const SPAN_MS = { last_hour: 3600e3, last_24h: 86400e3 };
const system = { timer: null };

/// Bytes as a person reads them.
function size(bytes) {
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 100 || unit === 0 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

const percent = (value) => `${value < 10 ? value.toFixed(1) : Math.round(value)}%`;

function svg(tag, attributes) {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
  return node;
}

const clock = (ms) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });

/// A key to a graph's lines.
function keys(lines) {
  const list = el("div", "system-keys");
  for (const line of lines) {
    const key = el("span", `key ${line.kind}`);
    key.append(el("i"), line.label);
    list.append(key);
  }
  return list;
}

/// A graph of `lines` over the window ending now, from 0 to `max`, read out where it is pointed at.
/// Each line is `{ label, kind, value(sample), fill }`; `show` writes a value as a person reads it.
function graph(report, max, lines, show) {
  const end = Date.now();
  const start = end - SPAN_MS[report.window];
  const x = (ms) => ((ms - start) / (end - start)) * GRAPH.w;
  const y = (value) => GRAPH.h - (Math.min(Math.max(value, 0), max) / max) * GRAPH.h;
  const times = report.samples.map((sample) => Date.parse(sample.at));
  // A gap of more than three readings is the machine asleep or the process stopped: the line
  // breaks there, rather than drawing a straight line across time nothing was read.
  const gap = report.every_sec * 3000;

  const drawing = svg("svg", { viewBox: `0 0 ${GRAPH.w} ${GRAPH.h}`, preserveAspectRatio: "none" });
  for (const fraction of [0.25, 0.5, 0.75]) {
    const height = GRAPH.h * fraction;
    drawing.append(svg("line", { x1: 0, x2: GRAPH.w, y1: height, y2: height, class: "grid" }));
  }
  for (const line of lines) {
    let trace = "";
    let area = "";
    times.forEach((at, index) => {
      const px = x(at).toFixed(1);
      const py = y(line.value(report.samples[index])).toFixed(1);
      if (index === 0 || at - times[index - 1] > gap) {
        if (index > 0) area += `L${x(times[index - 1]).toFixed(1)},${GRAPH.h}Z`;
        trace += `M${px},${py}`;
        area += `M${px},${GRAPH.h}L${px},${py}`;
      } else {
        trace += `L${px},${py}`;
        area += `L${px},${py}`;
      }
    });
    if (times.length > 0) area += `L${x(times[times.length - 1]).toFixed(1)},${GRAPH.h}Z`;
    if (line.fill) drawing.append(svg("path", { d: area, class: `area ${line.kind}` }));
    drawing.append(svg("path", { d: trace, class: `trace ${line.kind}` }));
  }
  const cursor = svg("line", { y1: 0, y2: GRAPH.h, class: "cursor", visibility: "hidden" });
  drawing.append(cursor);

  const box = el("div", "graph");
  const readout = el("div", "readout");
  readout.hidden = true;
  box.append(drawing, el("span", "axis", show(max)), readout);

  drawing.addEventListener("pointermove", (event) => {
    if (times.length === 0) return;
    const rect = drawing.getBoundingClientRect();
    const pointed = start + ((event.clientX - rect.left) / rect.width) * (end - start);
    let nearest = 0;
    times.forEach((at, index) => {
      if (Math.abs(at - pointed) < Math.abs(times[nearest] - pointed)) nearest = index;
    });
    const sample = report.samples[nearest];
    const px = x(times[nearest]);
    cursor.setAttribute("x1", px);
    cursor.setAttribute("x2", px);
    cursor.setAttribute("visibility", "visible");
    readout.replaceChildren(
      el("b", "", new Date(times[nearest]).toLocaleTimeString([], { hour12: false })),
      ...lines.map((line) => {
        const row = el("span", `key ${line.kind}`);
        row.append(el("i"), `${line.label} ${show(line.value(sample))}`);
        return row;
      }),
    );
    readout.style.left = `${(px / GRAPH.w) * 100}%`;
    readout.classList.toggle("flip", px > GRAPH.w * 0.6);
    readout.hidden = false;
  });
  drawing.addEventListener("pointerleave", () => {
    cursor.setAttribute("visibility", "hidden");
    readout.hidden = true;
  });

  const ticks = el("div", "ticks");
  ticks.append(el("span", "", clock(start)), el("span", "", clock((start + end) / 2)), el("span", "", "now"));
  const wrap = el("div", "graph-wrap");
  wrap.append(box, ticks);
  return wrap;
}

/// A card: what it measures, the latest figure and what it is out of, a line saying more, and the
/// graph.
function systemCard(title, [figure, outOf], note, lines, drawn, alarming = false) {
  const card = el("div", "card system-card");
  const head = el("div", "system-head");
  const figures = el("div");
  const value = el("span", `v${alarming ? " alarming" : ""}`, figure);
  if (outOf) value.append(el("small", "", outOf));
  figures.append(el("span", "k", title), value);
  head.append(figures, keys(lines));
  card.append(head, el("span", "n", note), drawn);
  return card;
}

function drawSystem(report) {
  const last = report.samples[report.samples.length - 1];
  const machine = { label: "Machine", kind: "machine", fill: true };
  const layover = { label: "Layover", kind: "layover" };

  const cpu = [
    { ...machine, value: (sample) => sample.cpu_percent },
    { ...layover, value: (sample) => sample.layover_cpu_percent },
  ];
  const memory = [
    { ...machine, value: (sample) => sample.memory_used_bytes },
    { ...layover, value: (sample) => sample.layover_memory_bytes },
  ];
  const cards = [
    systemCard(
      "Processors",
      [percent(last.cpu_percent)],
      `Layover ${percent(last.layover_cpu_percent)} · ${report.cpus} logical processors`,
      cpu,
      graph(report, 100, cpu, percent),
      last.cpu_percent >= 90,
    ),
    systemCard(
      "Memory",
      [size(last.memory_used_bytes), `of ${size(report.memory_total_bytes)}`],
      `Layover ${size(last.layover_memory_bytes)} in ${last.layover_processes} process${last.layover_processes === 1 ? "" : "es"}`,
      memory,
      graph(report, report.memory_total_bytes, memory, size),
      last.memory_used_bytes >= report.memory_total_bytes * 0.9,
    ),
  ];
  report.disks.forEach((disk, index) => {
    const used = [{ label: "Used", kind: "machine", fill: true, value: (sample) => sample.disk_used_bytes[index] }];
    const now = last.disk_used_bytes[index];
    const holds = disk.holds.map((what) => (what === "work_dir" ? "the work_dir" : "the factory")).join(" and ");
    cards.push(
      systemCard(
        `Disk ${disk.mount_point}`,
        [size(now), `of ${size(disk.total_bytes)}`],
        `${size(disk.total_bytes - now)} free · holds ${holds}`,
        used,
        graph(report, disk.total_bytes, used, size),
        now >= disk.total_bytes * 0.9,
      ),
    );
  });
  $("#system-graphs").replaceChildren(...cards);
}

async function loadSystem() {
  const span = $("#system-window").value;
  const none = $("#system-none");
  try {
    const report = await get(`/system?window=${span}`);
    const since = Date.parse(report.since);
    $("#system-since").textContent =
      since > Date.now() - SPAN_MS[report.window]
        ? `Since ${clock(since)}, when this dashboard started: it keeps what it reads in memory.`
        : "";
    if (report.samples.length === 0) {
      $("#system-graphs").replaceChildren();
      none.textContent = `Nothing read yet: the first reading comes within ${report.every_sec} seconds of the dashboard starting.`;
      none.hidden = false;
    } else {
      none.hidden = true;
      drawSystem(report);
    }
    updated("#system-updated");
  } catch (error) {
    $("#system-graphs").replaceChildren();
    none.textContent = `Could not read the machine: ${error.message}`;
    none.hidden = false;
  }
}

// Kept current only while it is being looked at: every reading for the last hour, every minute's
// average for the last day.
function watchSystem(visible) {
  clearInterval(system.timer);
  system.timer = null;
  if (!visible) return;
  loadSystem();
  const every = $("#system-window").value === "last_24h" ? 60000 : 5000;
  system.timer = setInterval(() => {
    if (!document.hidden) loadSystem();
  }, every);
}

function startSystem() {
  $("#system-window").addEventListener("change", () => {
    routeChanged();
    watchSystem(true);
  });
}
