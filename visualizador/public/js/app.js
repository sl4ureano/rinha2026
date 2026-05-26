import { createScene, PATH_COLORS } from "./scene3d.js";

const PATH_LABELS = {
  ObviousLegit: "Gasto seguro — atalho",
  ObviousFraud: "Gasto arriscado — atalho",
  Tree: "Árvore de decisão",
  Ratio: "Fallback ratio",
};

const wrap = document.getElementById("canvas-wrap");
const scene3d = createScene(wrap);

const $ = (id) => document.getElementById(id);
const payloadEl = $("payload");
const examplesEl = $("examples");

async function loadExamples() {
  const r = await fetch("/api/examples");
  const list = await r.json();
  examplesEl.innerHTML = '<option value="">— escolha —</option>';
  list.forEach((ex, i) => {
    const opt = document.createElement("option");
    opt.value = String(i);
    opt.textContent = `${ex.id} — R$ ${ex.transaction.amount}`;
    examplesEl.appendChild(opt);
  });
  window.__examples = list;
  if (list[0]) payloadEl.value = JSON.stringify(list[0], null, 2);
}

examplesEl.addEventListener("change", () => {
  const i = examplesEl.value;
  if (i === "") return;
  payloadEl.value = JSON.stringify(window.__examples[Number(i)], null, 2);
});

function renderMetrics(event) {
  const { trace, proxy, match } = event;
  $("metrics-empty").classList.add("hidden");
  $("metrics").classList.remove("hidden");

  const badge = $("path-badge");
  badge.textContent = PATH_LABELS[trace.path] ?? trace.path;
  badge.className = `metric-hero ${trace.path.replace("Obvious", "").toLowerCase()}`;

  $("m-api").textContent = trace.api;
  $("m-path").textContent = trace.path;
  $("m-count").textContent = String(trace.fraudCount);
  $("m-response").textContent = JSON.stringify(trace.response);
  $("m-trace-ms").textContent = `${trace.timingMs.classify.toFixed(2)} ms`;
  $("m-proxy-ms").textContent = proxy
    ? proxy.ok
      ? `${proxy.timingMs.toFixed(2)} ms (${proxy.status})`
      : `offline — ${proxy.error || "?"}`
    : "— (simulação)";
  $("m-match").textContent = match === null ? "—" : match ? "✓ igual" : "✗ diverge";

  const checks = $("checks-block");
  checks.innerHTML = "";
  const checkSections =
    trace.path === "ObviousLegit"
      ? [["Gasto seguro", trace.checks.obviousLegit]]
      : trace.path === "ObviousFraud"
        ? [["Gasto arriscado", trace.checks.obviousFraud]]
        : [
            ["Gasto seguro (não)", trace.checks.obviousLegit],
            ["Gasto arriscado (não)", trace.checks.obviousFraud],
          ];
  for (const [name, block] of checkSections) {
    const h = document.createElement("div");
    h.className = "check-list";
    h.innerHTML = `<h3>${name} ${block.pass ? "✓" : "—"}</h3>`;
    for (const c of block.checks) {
      const row = document.createElement("div");
      row.className = `check-item ${c.ok ? "ok" : "fail"}`;
      row.innerHTML = `<span class="dot"></span><span>${c.label}</span>`;
      h.appendChild(row);
    }
    checks.appendChild(h);
  }

  const treeBlock = $("tree-block");
  if (trace.tree?.walk?.length) {
    treeBlock.classList.remove("hidden");
    treeBlock.innerHTML = `<h3 style="font-size:0.75rem;color:var(--muted)">Árvore (${trace.tree.walk.length} passos)</h3><div class="tree-walk">${trace.tree.walk
      .map((w) =>
        w.leaf
          ? `→ folha: ${w.fraud ? "FRAUDE" : "OK"}`
          : `[${w.depth}] ${w.featureLabel}: ${w.value.toFixed(4)} ${w.branch === "left" ? "≤" : ">"} ${w.threshold.toFixed(4)}`,
      )
      .join("\n")}</div>`;
  } else {
    treeBlock.classList.add("hidden");
  }
}

async function sendRequest(simulateOnly) {
  let body;
  try {
    body = JSON.parse(payloadEl.value);
  } catch {
    alert("JSON inválido");
    return;
  }
  const url = simulateOnly ? "/api/simulate" : "/api/trace";
  const btn = simulateOnly ? $("btn-sim") : $("btn-send");
  btn.disabled = true;
  try {
    const r = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    const event = await r.json();
    if (event.error) throw new Error(event.error);
    renderMetrics(event);
    scene3d.animateFlow(event);
  } catch (e) {
    alert(e.message || String(e));
  } finally {
    btn.disabled = false;
  }
}

$("btn-send").addEventListener("click", () => sendRequest(false));
$("btn-sim").addEventListener("click", () => sendRequest(true));

const es = new EventSource("/api/events");
es.onopen = () => {
  $("sse-status").textContent = "SSE: conectado";
  $("sse-status").className = "pill ok";
};
es.onerror = () => {
  $("sse-status").textContent = "SSE: reconectando…";
  $("sse-status").className = "pill warn";
};
es.onmessage = (ev) => {
  const data = JSON.parse(ev.data);
  if (data.type === "flow") {
    renderMetrics(data);
    scene3d.animateFlow(data);
  }
};

async function checkApi() {
  const pill = $("api-status");
  try {
    const h = await fetch("/api/health");
    const j = await h.json();
    pill.textContent = `API alvo: ${j.fraudApi}`;
    pill.className = "pill ok";
  } catch {
    pill.textContent = "API: erro";
    pill.className = "pill err";
  }
}

$("legend").innerHTML = Object.entries(PATH_COLORS)
  .map(([k, c]) => {
    const hex = "#" + c.toString(16).padStart(6, "0");
    return `<span><i style="background:${hex}"></i>${PATH_LABELS[k]}</span>`;
  })
  .join("");

loadExamples();
checkApi();
addEventListener("resize", () => {
  scene3d.onResize();
  scene3d.fitCamera?.();
});

function loop() {
  scene3d.render();
  requestAnimationFrame(loop);
}
loop();
