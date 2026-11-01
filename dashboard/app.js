// Thornhill Ambient Web Console — app.js (Faz 6)
// Saf Vanilla JS; yalnız yerel SSE + REST (RFC-001 §7 ile aynı tel tipleri).
"use strict";

// ---------- Palet ----------
const C = {
  alarm: "#e06c75",
  sense: "#61afef",
  reflex: "#98c379",
  cogn: "#e0af68",
  dim: "#5c6370",
  border: "#3b4261",
};

// ---------- Yardımcılar ----------
const $ = (id) => document.getElementById(id);
const fmtTime = (ms) => new Date(ms).toLocaleTimeString("tr-TR");
const escapeHtml = (s) =>
  String(s).replace(/[&<>"']/g, (ch) =>
    ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[ch])
  );

function thFetch(url, init = {}) {
  init.headers = init.headers || {};
  if (window.__thToken) {
    init.headers["X-Thornhill-Token"] = window.__thToken;
  }
  return fetch(url, init);
}

// ---------- Telemetri: statik sayaçlar ----------
async function refreshStats() {
  try {
    const r = await thFetch("/api/stats");
    const s = await r.json();
    $("m-events").textContent = s.events ?? "—";
    $("m-edges").textContent = s.edges ?? "—";
    $("m-lessons").textContent = s.lessons ?? "—";
    $("m-habits").textContent = s.habits ?? "—";
  } catch (e) {
    console.warn("stats alınamadı:", e);
  }
}

// ---------- Olay akış grafiği (son 100 olay) ----------
// DÜRÜSTLÜK NOTU: Tarayıcı daemon'un G3 boru hattı süresini ölçEMEZ; otorite
// ölçüm daemon'un kendi log'undadır ("G3 boru hattı" uyarıları, Faz 5).
// Burada çizilen GERÇEK ölçülebilir veridir: olaylar arası varış aralığı (ms).
const latencyHistory = []; // {ts, ms}
const G3_BUDGET_MS = 1.0;
let lastArrivalTs = null;

function drawLatency() {
  const svg = d3.select("#latency");
  svg.selectAll("*").remove();
  const W = 760, H = 120, pad = 6;
  const data = latencyHistory.slice(-100);
  if (data.length < 2) return;

  const x = d3.scaleLinear().domain([0, data.length - 1]).range([pad, W - pad]);
  const maxMs = Math.max(G3_BUDGET_MS * 1.2, d3.max(data, (d) => d.ms) || 1);
  const y = d3.scaleLinear().domain([0, maxMs]).range([H - pad, pad]);

  // Bütçe çizgisi (1 ms)
  svg.append("line")
    .attr("x1", pad).attr("x2", W - pad)
    .attr("y1", y(G3_BUDGET_MS)).attr("y2", y(G3_BUDGET_MS))
    .attr("stroke", C.reflex).attr("stroke-dasharray", "4 4")
    .attr("stroke-opacity", 0.7);

  const line = d3.line()
    .x((d, i) => x(i))
    .y((d) => y(Math.min(d.ms, maxMs)));

  svg.append("path")
    .datum(data)
    .attr("d", line)
    .attr("fill", "none")
    .attr("stroke", C.sense)
    .attr("stroke-width", 1.5);

  // Aşımlar kırmızı nokta
  svg.selectAll("circle.over").data(data).join("circle")
    .attr("class", "over")
    .attr("cx", (d, i) => x(i))
    .attr("cy", (d) => y(Math.min(d.ms, maxMs)))
    .attr("r", (d) => (d.ms > G3_BUDGET_MS ? 3 : 0))
    .attr("fill", C.alarm);

  // p50/p99 — olaylar arası varış aralığı üzerinden (G3 otoritesi daemon log'u)
  const sorted = data.map((d) => d.ms).sort((a, b) => a - b);
  const p = (q) => sorted[Math.floor(q * (sorted.length - 1))] || 0;
  $("m-p50").textContent = p(0.5).toFixed(1) + " ms aralık";
  $("m-p99").textContent = p(0.99).toFixed(1) + " ms aralık";
  $("m-p50").className = "value";
  $("m-p99").className = "value";
}

// ---------- D3 Nedensellik Grafiği ----------
// Düğümler kind bazında birleşir; kenar kalınlığı support'a göre.
let graphNodes = [], graphLinks = [];
const kindColor = (kind) =>
  kind.startsWith("cmd.") ? C.alarm
  : kind.startsWith("s2") || kind.startsWith("chip") ? C.cogn
  : kind.startsWith("win") || kind.startsWith("ws") ? C.sense
  : C.reflex;

async function refreshGraph() {
  try {
    const [evRes, edgeRes] = await Promise.all([
      thFetch("/api/query?q=recent&limit=100").then((r) => r.json()),
      thFetch("/api/query?q=causal&limit=60").then((r) => r.json()),
    ]);

    const counts = {}; // kind -> adet
    (evRes.rows || []).forEach((r) => { counts[r.kind] = (counts[r.kind] || 0) + 1; });

    const nodesSet = new Set();
    graphLinks = (edgeRes.rows || []).map((e) => {
      nodesSet.add(e.src_kind);
      nodesSet.add(e.dst_kind);
      return { source: e.src_kind, target: e.dst_kind, support: e.support };
    });
    Object.keys(counts).forEach((k) => nodesSet.add(k));

    graphNodes = Array.from(nodesSet).map((kind) => ({
      id: kind, count: counts[kind] || 0,
    }));

    drawGraph();
  } catch (e) {
    console.warn("graf yenilenemedi:", e);
  }
}

function drawGraph() {
  const svg = d3.select("#graph");
  svg.selectAll("*").remove();
  const W = 760, H = 460;

  const sim = d3.forceSimulation(graphNodes)
    .force("link", d3.forceLink(graphLinks).id((d) => d.id).distance(110))
    .force("charge", d3.forceManyBody().strength(-260))
    .force("center", d3.forceCenter(W / 2, H / 2))
    .force("collide", d3.forceCollide(34));

  const g = svg.append("g");

  const link = g.selectAll("line")
    .data(graphLinks).join("line")
    .attr("class", "link caused")
    .attr("stroke-width", (d) => 1 + Math.min(5, d.support));

  const node = g.selectAll("g.node")
    .data(graphNodes).join("g")
    .attr("class", "node")
    .style("cursor", "pointer")
    .call(d3.drag()
      .on("start", (e, d) => { if (!e.active) sim.alphaTarget(0.3).restart(); d.fx = d.x; d.fy = d.y; })
      .on("drag", (e, d) => { d.fx = e.x; d.fy = e.y; })
      .on("end", (e, d) => { if (!e.active) sim.alphaTarget(0); d.fx = null; d.fy = null; }));

  node.append("circle")
    .attr("r", (d) => 12 + Math.min(14, d.count * 1.5))
    .attr("fill", (d) => kindColor(d.id))
    .attr("fill-opacity", 0.25)
    .attr("stroke", (d) => kindColor(d.id));

  node.append("text")
    .text((d) => d.id + (d.count ? " ×" + d.count : ""))
    .attr("text-anchor", "middle")
    .attr("dy", "2.6em");

  node.on("mouseenter", function (e, d) {
      link.attr("stroke-opacity", (l) =>
        l.source.id === d.id || l.target.id === d.id ? 1 : 0.12);
    })
    .on("mouseleave", () => link.attr("stroke-opacity", 0.55));

  sim.on("tick", () => {
    link.attr("x1", (d) => d.source.x).attr("y1", (d) => d.source.y)
        .attr("x2", (d) => d.target.x).attr("y2", (d) => d.target.y);
    node.attr("transform", (d) => `translate(${d.x},${d.y})`);
  });
}

// ---------- SSE: canlı olay ve çip akışı ----------
let es = null;
function initEventSource() {
  const streamUrl = window.__thToken
    ? "/api/stream?token=" + encodeURIComponent(window.__thToken)
    : "/api/stream";
  es = new EventSource(streamUrl);
  es.onopen = () => {
    $("eye").textContent = "👁️";
    $("eye").classList.add("awake");
  };
  es.onerror = () => {
    $("eye").textContent = "💤";
    $("eye").classList.remove("awake");
  };
  es.onmessage = onSseMessage;
}

function onSseMessage(ev) {
  let msg;
  try { msg = JSON.parse(ev.data); } catch { return; }

  if (msg.op === "event.push" && msg.event) {
    pushTick(msg.event);
    const now = Date.now();
    const gap = lastArrivalTs === null ? null : now - lastArrivalTs;
    lastArrivalTs = now;
    if (gap !== null) {
      latencyHistory.push({ ts: now, ms: gap });
      if (latencyHistory.length > 120) latencyHistory.shift();
      drawLatency();
    }
    refreshStats();
  } else if (msg.op === "chip.push" && msg.chip) {
    addChip(msg.chip);
  } else if (msg.op === "stream.lagged") {
    pushSystemTick(`akış ${msg.missed} olay kaçırdı (coalesce)`);
  }
};

function pushTick(e) {
  const isErr = e.kind === "cmd.done" && e.ctx && e.ctx.exit_code !== 0;
  const div = document.createElement("div");
  div.className = "tick" + (isErr ? " err" : "");
  div.innerHTML =
    `<span class="t">${fmtTime(e.ts)}</span> ` +
    `<span class="src">${escapeHtml(e.src)}</span> ` +
    `<span class="kind">${escapeHtml(e.kind)}</span> ` +
    `<span>${escapeHtml(e.subject)}</span>` +
    (isErr ? ` <span class="alarm">exit ${escapeHtml(e.ctx.exit_code)}</span>` : "");
  const ticker = $("ticker");
  ticker.prepend(div);
  while (ticker.children.length > 100) ticker.lastChild.remove();
}

function pushSystemTick(text) {
  const div = document.createElement("div");
  div.className = "tick";
  div.innerHTML = `<span class="t">${fmtTime(Date.now())}</span> <span class="alarm">sistem</span> <span>${escapeHtml(text)}</span>`;
  $("ticker").prepend(div);
}

// ---------- Action Chip listesi ----------
const chipStates = {}; // id -> accepted|refused|expired

function addChip(chip) {
  chipStates[chip.id] = "live";
  const wrap = document.createElement("div");
  wrap.className = "chip";
  wrap.dataset.chipId = chip.id;

  const exec = (chip.actions || []).find((a) => a.action_type === "exec");
  wrap.innerHTML =
    `<h3>${escapeHtml(chip.title)}</h3>` +
    `<p class="why">${escapeHtml(chip.why)}</p>` +
    (exec ? `<p class="cmd">⚑ ${escapeHtml(exec.payload?.cmd || "")}</p>` : "") +
    `<div class="btns"></div>`;

  const btns = wrap.querySelector(".btns");
  (chip.actions || []).forEach((a) => {
    const b = document.createElement("button");
    b.textContent = a.label + (a.risk === "medium" ? " ⏳" : "");
    b.onclick = () => chipAct(chip.id, a.label, a.risk !== "none", wrap);
    btns.appendChild(b);
  });

  $("chips").prepend(wrap);
  const hint = $("chips").querySelector(".hint");
  if (hint) hint.remove();
}

async function chipAct(chipId, actionLabel, needsConfirm, wrapEl) {
  if (needsConfirm && !confirm(`Onay: "${actionLabel}" eylemi çalıştırılsın mı?`)) {
    chipStates[chipId] = "refused";
    markChip(wrapEl, "reddedildi");
    return;
  }
  try {
    const r = await thFetch("/api/chip-act", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Thornhill-Token": window.__thToken || "" },
      body: JSON.stringify({ chip_id: chipId, action_label: actionLabel, confirm: true }),
    });
    const j = await r.json();
    if (r.ok && j.dispatched) {
      chipStates[chipId] = "accepted";
      markChip(wrapEl, "onaylandı ✓");
    } else {
      chipStates[chipId] = "refused";
      markChip(wrapEl, "reddedildi: " + (j.error || r.status));
    }
  } catch (e) {
    markChip(wrapEl, "hata: " + e);
  }
}

function markChip(wrapEl, status) {
  if (!wrapEl) return;
  const s = document.createElement("p");
  s.className = "why";
  s.innerHTML = `<span class="c-cogn">→ ${escapeHtml(status)}</span>`;
  wrapEl.querySelectorAll("button").forEach((b) => (b.disabled = true));
  wrapEl.appendChild(s);
}

// ---------- S2 Ask konsolu ----------
$("ask-form").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const q = $("ask-input").value.trim();
  if (!q) return;
  $("ask-input").value = "";
  const log = $("s2-log");
  log.insertAdjacentHTML("beforeend", `<div class="q">▶ ${escapeHtml(q)}</div><div class="a" id="pending">…düşünüyor</div>`);
  try {
    const r = await thFetch("/api/ask", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ question: q }),
    });
    const j = await r.json();
    document.getElementById("pending")?.remove();
    if (r.ok) {
      log.insertAdjacentHTML("beforeend",
        `<div class="a">${escapeHtml(j.explanation)}${j.remedy ? "\n→ " + escapeHtml(j.remedy) : ""}</div>`);
    } else {
      log.insertAdjacentHTML("beforeend", `<div class="err">✗ ${escapeHtml(j.error || r.status)}</div>`);
    }
  } catch (e) {
    document.getElementById("pending")?.remove();
    log.insertAdjacentHTML("beforeend", `<div class="err">✗ bağlantı hatası: ${escapeHtml(e)}</div>`);
  }
  log.scrollTop = log.scrollHeight;
});

// ---------- Unutma hakkı ----------
document.querySelectorAll("button.forget").forEach((b) => {
  b.addEventListener("click", async () => {
    const days = parseInt(b.dataset.days, 10);
    const beforeTs = Date.now() - days * 86400000;
    if (!confirm(`${days} günden eski olaylar silinsin mi? (lessons/habits korunur)`)) return;
    try {
      const r = await thFetch("/api/forget", {
        method: "POST",
        headers: { "Content-Type": "application/json", "X-Thornhill-Token": window.__thToken || "" },
        body: JSON.stringify({ before_ts: beforeTs }),
      });
      const j = await r.json();
      alert(r.ok
        ? `🧹 ${j.forgotten_events} olay, ${j.forgotten_edges} kenar silindi.`
        : `Hata: ${j.error || r.status}`);
      refreshStats();
      refreshGraph();
    } catch (e) {
      alert("bağlantı hatası: " + e);
    }
  });
});

// ---------- Başlatma ----------
(async function init() {
  const urlParams = new URLSearchParams(window.location.search);
  const qToken = urlParams.get("token");
  if (qToken) {
    sessionStorage.setItem("thornhill_token", qToken);
  }
  window.__thToken = sessionStorage.getItem("thornhill_token") || "";
  if (!window.__thToken) {
    const entered = prompt("Thornhill erişim token'ını giriniz:\n(Token dosyası: $XDG_RUNTIME_DIR/thornhill/http.token)");
    if (entered) {
      window.__thToken = entered.trim();
      sessionStorage.setItem("thornhill_token", window.__thToken);
    }
  }
  initEventSource();
  refreshStats();
  refreshGraph();
  setInterval(refreshStats, 5000);
  setInterval(refreshGraph, 15000);
})();
