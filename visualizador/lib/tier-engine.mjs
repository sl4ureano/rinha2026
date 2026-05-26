const LEAF = 255;
const FEATURE_COUNT = 21;
const MAX_AMOUNT = 10_000;
const MAX_INSTALLMENTS = 12;
const AMOUNT_VS_AVG_RATIO = 10;
const MAX_MINUTES = 1440;
const MAX_KM = 1000;
const MAX_TX24H = 20;
const MAX_MERCHANT_AVG = 10_000;
const RATIO_FRAUD_THRESHOLD = 0.06951915;
const LEGIT_RATIO_CAP = 0.50001;

export const FEATURE_LABELS = [
  "amount / 10k",
  "parcelas / 12",
  "ratio amount/avg / 10",
  "hora / 23",
  "dia da semana / 6",
  "min desde última tx",
  "km desde última tx",
  "km de casa / 1000",
  "tx 24h / 20",
  "online",
  "cartão presente",
  "loja desconhecida",
  "risco MCC",
  "média loja / 10k",
  "sem última tx",
  "amount (raw)",
  "média cliente (raw)",
  "ratio (raw)",
  "tx 24h (raw)",
  "km casa (raw)",
  "média loja (raw)",
];

const SAFE_MCC = new Set(["5411", "5812", "5912", "5311"]);
const RISKY_MCC = new Set(["7995", "7801", "7802"]);

const MCC_RISK = {
  5411: 0.15,
  5812: 0.3,
  5912: 0.2,
  5944: 0.45,
  7801: 0.8,
  7802: 0.75,
  7995: 0.85,
  4511: 0.35,
  5311: 0.25,
  5999: 0.5,
};

function clamp01(x) {
  if (x < 0) return 0;
  if (x > 1) return 1;
  return x;
}

function mccRisk(mcc) {
  return MCC_RISK[mcc] ?? 0.5;
}

function merchantKnown(merchantId, knownMerchants) {
  if (!merchantId || !Array.isArray(knownMerchants)) return false;
  return knownMerchants.includes(merchantId);
}

function digit2(a, b) {
  if (a >= "0" && a <= "9" && b >= "0" && b <= "9") {
    return (a.charCodeAt(0) - 48) * 10 + (b.charCodeAt(0) - 48);
  }
  return null;
}

function digit4(s, i) {
  const a = digit2(s[i], s[i + 1]);
  const b = digit2(s[i + 2], s[i + 3]);
  if (a === null || b === null) return null;
  return a * 100 + b;
}

function daysFromCivil(y, m, d) {
  let year = y;
  let month = m;
  if (month <= 2) year -= 1;
  const era = Math.floor((year >= 0 ? year : year - 399) / 400);
  const yoe = year - era * 400;
  const monthAdj = month > 2 ? month - 3 : month + 9;
  const doy = Math.floor((153 * monthAdj + 2) / 5) + d - 1;
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy;
  return era * 146097 + doe - 719468;
}

function parseIso(ts) {
  if (!ts || ts.length < 19) return null;
  const year = digit4(ts, 0);
  if (year === null || ts[4] !== "-" || ts[7] !== "-" || ts[10] !== "T" || ts[13] !== ":") {
    return null;
  }
  const month = digit2(ts[5], ts[6]);
  const day = digit2(ts[8], ts[9]);
  const hour = digit2(ts[11], ts[12]);
  const minute = digit2(ts[14], ts[15]);
  const second = digit2(ts[17], ts[18]) ?? 0;
  if ([month, day, hour, minute, second].some((v) => v === null)) return null;
  const days = daysFromCivil(year, month, day);
  const weekday = ((days + 3) % 7 + 7) % 7;
  return {
    hour,
    weekdayMonday0: weekday,
    epochSeconds: days * 86400 + hour * 3600 + minute * 60 + second,
  };
}

function extractPayload(body) {
  if (!body || typeof body !== "object") return null;
  const tx = body.transaction ?? {};
  const cust = body.customer ?? {};
  const merch = body.merchant ?? {};
  const term = body.terminal ?? {};
  const last = body.last_transaction;
  return {
    id: body.id ?? "—",
    amount: Number(tx.amount) || 0,
    installments: Number(tx.installments) || 0,
    requestedAt: tx.requested_at ?? null,
    customerAvgAmount: Number(cust.avg_amount) || 0,
    txCount24h: Number(cust.tx_count_24h) || 0,
    knownMerchants: Array.isArray(cust.known_merchants) ? cust.known_merchants : [],
    merchantId: merch.id ?? "",
    merchantMcc: String(merch.mcc ?? ""),
    merchantAvgAmount: Number(merch.avg_amount) || 0,
    isOnline: Boolean(term.is_online),
    cardPresent: Boolean(term.card_present),
    kmFromHome: Number(term.km_from_home) || 0,
    lastTimestamp: last?.timestamp ?? null,
    lastKm: last?.km_from_current != null ? Number(last.km_from_current) : null,
  };
}

function checkObviousLegit(p, ctx) {
  const checks = [
    { id: "amount", label: "valor ≤ 500", ok: p.amount <= 500, value: p.amount },
    {
      id: "ratio",
      label: "valor ≤ 50% da média",
      ok: p.amount <= ctx.safeAvg * LEGIT_RATIO_CAP,
      value: p.amount / ctx.safeAvg,
    },
    { id: "installments", label: "≤ 3 parcelas", ok: p.installments <= 3, value: p.installments },
    { id: "tx24h", label: "≤ 5 tx/24h", ok: p.txCount24h <= 5, value: p.txCount24h },
    { id: "km", label: "≤ 50 km de casa", ok: p.kmFromHome <= 50, value: p.kmFromHome },
    { id: "mcc", label: "MCC seguro", ok: SAFE_MCC.has(ctx.mcc), value: ctx.mcc },
    { id: "known", label: "loja conhecida", ok: ctx.known, value: p.merchantId },
  ];
  return { pass: checks.every((c) => c.ok), checks };
}

function checkObviousFraud(p, ctx) {
  const checks = [
    { id: "amount", label: "valor ≥ 5000", ok: p.amount >= 5000, value: p.amount },
    { id: "installments", label: "≥ 5 parcelas", ok: p.installments >= 5, value: p.installments },
    { id: "tx24h", label: "≥ 6 tx/24h", ok: p.txCount24h >= 6, value: p.txCount24h },
    { id: "km", label: "≥ 150 km de casa", ok: p.kmFromHome >= 150, value: p.kmFromHome },
    { id: "mcc", label: "MCC arriscado", ok: RISKY_MCC.has(ctx.mcc), value: ctx.mcc },
    { id: "unknown", label: "loja desconhecida", ok: !ctx.known, value: p.merchantId },
  ];
  return { pass: checks.every((c) => c.ok), checks };
}

function buildTreeFeatures(p, ctx) {
  const requested = ctx.requested;
  if (!requested) return null;
  const amountRatio = p.amount / ctx.safeAvg;
  let minutesSinceLast = -1;
  let kmFromLast = -1;
  let lastNull = 1;
  if (p.lastTimestamp) {
    const last = parseIso(p.lastTimestamp);
    if (!last) return null;
    const delta = requested.epochSeconds - last.epochSeconds;
    minutesSinceLast = clamp01(Math.max(0, delta) / 60 / MAX_MINUTES);
    kmFromLast = p.lastKm != null ? clamp01(p.lastKm / MAX_KM) : -1;
    lastNull = 0;
  }
  return [
    clamp01(p.amount / MAX_AMOUNT),
    clamp01(p.installments / MAX_INSTALLMENTS),
    clamp01(amountRatio / AMOUNT_VS_AVG_RATIO),
    requested.hour / 23,
    requested.weekdayMonday0 / 6,
    minutesSinceLast,
    kmFromLast,
    clamp01(p.kmFromHome / MAX_KM),
    clamp01(p.txCount24h / MAX_TX24H),
    p.isOnline ? 1 : 0,
    p.cardPresent ? 1 : 0,
    ctx.known ? 0 : 1,
    mccRisk(ctx.mcc),
    clamp01(p.merchantAvgAmount / MAX_MERCHANT_AVG),
    lastNull,
    p.amount,
    p.customerAvgAmount,
    amountRatio,
    p.txCount24h,
    p.kmFromHome,
    p.merchantAvgAmount,
  ];
}

function predictTree(nodes, features) {
  const walk = [];
  let index = 0;
  for (let depth = 0; depth < 64; depth++) {
    const node = nodes[index];
    if (node.feature === LEAF) {
      walk.push({
        depth,
        index,
        leaf: true,
        fraud: node.fraud,
      });
      return { fraud: node.fraud, walk };
    }
    const v = features[node.feature];
    const goLeft = v <= node.threshold;
    const next = goLeft ? node.left : node.right;
    walk.push({
      depth,
      index,
      feature: node.feature,
      featureLabel: FEATURE_LABELS[node.feature],
      value: v,
      threshold: node.threshold,
      branch: goLeft ? "left" : "right",
      next,
    });
    index = next;
  }
  throw new Error("árvore excedeu profundidade");
}

function ratioFraudCount(p) {
  const safeAvg = Math.max(p.customerAvgAmount, 1);
  const norm = clamp01(p.amount / safeAvg / AMOUNT_VS_AVG_RATIO);
  return {
    count: norm > RATIO_FRAUD_THRESHOLD ? 5 : 0,
    norm,
    threshold: RATIO_FRAUD_THRESHOLD,
    safeAvg,
  };
}

function countToResponse(count) {
  const map = {
    0: { approved: true, fraud_score: 0 },
    1: { approved: true, fraud_score: 0.2 },
    2: { approved: true, fraud_score: 0.4 },
    3: { approved: false, fraud_score: 0.6 },
    4: { approved: false, fraud_score: 0.8 },
  };
  return map[count] ?? { approved: false, fraud_score: 1 };
}

/**
 * @param {object} body - JSON da transação
 * @param {import('./tree-loader.mjs').TreeNode[]} nodes
 * @param {{ api?: string }} opts
 */
export function traceRequest(body, nodes, opts = {}) {
  const t0 = performance.now();
  const payload = extractPayload(body);
  if (!payload) {
    return {
      ok: false,
      error: "JSON inválido ou campos ausentes",
      timingMs: { classify: performance.now() - t0 },
    };
  }

  const ctx = {
    safeAvg: Math.max(payload.customerAvgAmount, 1),
    known: merchantKnown(payload.merchantId, payload.knownMerchants),
    mcc: payload.merchantMcc,
    requested: parseIso(payload.requestedAt),
  };

  const legit = checkObviousLegit(payload, ctx);
  const fraud = checkObviousFraud(payload, ctx);

  let path;
  let fraudCount;
  let treeResult = null;
  let ratioResult = null;

  if (legit.pass) {
    path = "ObviousLegit";
    fraudCount = 0;
  } else if (fraud.pass) {
    path = "ObviousFraud";
    fraudCount = 5;
  } else if (!ctx.requested) {
    path = "Ratio";
    ratioResult = ratioFraudCount(payload);
    fraudCount = ratioResult.count;
  } else {
    const features = buildTreeFeatures(payload, ctx);
    if (features) {
      path = "Tree";
      treeResult = predictTree(nodes, features);
      fraudCount = treeResult.fraud ? 5 : 0;
    } else {
      path = "Ratio";
      ratioResult = ratioFraudCount(payload);
      fraudCount = ratioResult.count;
    }
  }

  const response = countToResponse(fraudCount);
  const flowSteps = buildFlowSteps(path, opts.api ?? "api1");

  return {
    ok: true,
    txId: payload.id,
    api: opts.api ?? "api1",
    path,
    fraudCount,
    response,
    payload: {
      amount: payload.amount,
      merchantId: payload.merchantId,
      mcc: ctx.mcc,
      known: ctx.known,
    },
    checks: { obviousLegit: legit, obviousFraud: fraud },
    tree: treeResult
      ? {
          features: buildTreeFeatures(payload, ctx),
          walk: treeResult.walk,
          fraud: treeResult.fraud,
        }
      : null,
    ratio: ratioResult,
    flowSteps,
    timingMs: { classify: performance.now() - t0 },
  };
}

function buildFlowSteps(path, api) {
  const ingress = [
    { id: "client", label: "Cliente / k6", layer: "edge" },
    { id: "lb", label: "Load Balancer :9999", layer: "infra", detail: "epoll + round-robin" },
    { id: "socket", label: "Unix socket (SCM_RIGHTS)", layer: "infra", detail: "/tmp/sockets" },
    { id: api, label: api.toUpperCase(), layer: "worker", detail: "fd_gateway → thread http-conn" },
    { id: "http", label: "HTTP parse", layer: "handler", detail: "POST /fraud-score" },
    { id: "extract", label: "JSON extract", layer: "handler" },
  ];
  const tail = [
    { id: "response", label: "Resposta HTTP estática", layer: "out" },
    { id: "client_out", label: "Cliente", layer: "edge" },
  ];

  const tierEntry = [
    ...ingress,
    { id: "tier", label: "tier_score", layer: "scorer" },
  ];

  if (path === "ObviousLegit") {
    return [
      ...tierEntry,
      { id: "legit", label: "Gasto seguro? → sim → aprova", layer: "hit", active: true },
      ...tail,
    ];
  }
  if (path === "ObviousFraud") {
    return [
      ...tierEntry,
      { id: "fraud", label: "Gasto arriscado? → sim → nega", layer: "hit", active: true },
      ...tail,
    ];
  }
  if (path === "Tree") {
    return [
      ...tierEntry,
      { id: "tree", label: "Árvore de decisão (21 features)", layer: "hit", active: true },
      ...tail,
    ];
  }
  return [
    ...tierEntry,
    { id: "ratio", label: "Ratio amount/avg", layer: "hit", active: true },
    ...tail,
  ];
}
