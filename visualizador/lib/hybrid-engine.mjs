const LEGIT_RATIO_CAP = 0.50001;
const SAFE_MCC = new Set(["5411", "5812", "5912", "5311"]);

function merchantKnown(merchantId, knownMerchants) {
  if (!merchantId || !Array.isArray(knownMerchants)) return false;
  return knownMerchants.includes(merchantId);
}

function extractPayload(body) {
  if (!body || typeof body !== "object") return null;
  const tx = body.transaction ?? {};
  const cust = body.customer ?? {};
  const merch = body.merchant ?? {};
  const term = body.terminal ?? {};
  return {
    id: body.id ?? "-",
    amount: Number(tx.amount) || 0,
    installments: Number(tx.installments) || 0,
    customerAvgAmount: Number(cust.avg_amount) || 0,
    txCount24h: Number(cust.tx_count_24h) || 0,
    knownMerchants: Array.isArray(cust.known_merchants) ? cust.known_merchants : [],
    merchantId: merch.id ?? "",
    merchantMcc: String(merch.mcc ?? ""),
    isOnline: Boolean(term.is_online),
    cardPresent: Boolean(term.card_present),
    kmFromHome: Number(term.km_from_home) || 0,
  };
}

function checkSafeSpend(p, ctx) {
  const checks = [
    { id: "amount", label: "valor <= 500", ok: p.amount <= 500, value: p.amount },
    {
      id: "amount_vs_avg",
      label: "valor <= 50% da media",
      ok: p.amount <= ctx.safeAvg * LEGIT_RATIO_CAP,
      value: p.amount / ctx.safeAvg,
    },
    { id: "installments", label: "<= 3 parcelas", ok: p.installments <= 3, value: p.installments },
    { id: "tx24h", label: "<= 5 tx/24h", ok: p.txCount24h <= 5, value: p.txCount24h },
    { id: "km", label: "<= 50 km de casa", ok: p.kmFromHome <= 50, value: p.kmFromHome },
    { id: "mcc", label: "MCC seguro", ok: SAFE_MCC.has(ctx.mcc), value: ctx.mcc },
    { id: "known", label: "loja conhecida", ok: ctx.known, value: p.merchantId },
  ];
  return { pass: checks.every((c) => c.ok), checks };
}

function responseToCount(response) {
  if (!response || typeof response !== "object") return null;
  const score = Number(response.fraud_score);
  if (!Number.isFinite(score)) return null;
  if (score <= 0) return 0;
  if (score <= 0.2) return 1;
  if (score <= 0.4) return 2;
  if (score <= 0.6) return 3;
  if (score <= 0.8) return 4;
  return 5;
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
 * @param {object} body - JSON da transacao
 * @param {{ api?: string, apiResponse?: object }} opts
 */
export function traceRequest(body, opts = {}) {
  const t0 = performance.now();
  const payload = extractPayload(body);
  if (!payload) {
    return {
      ok: false,
      error: "JSON invalido ou campos ausentes",
      timingMs: { classify: performance.now() - t0 },
    };
  }

  const ctx = {
    safeAvg: Math.max(payload.customerAvgAmount, 1),
    known: merchantKnown(payload.merchantId, payload.knownMerchants),
    mcc: payload.merchantMcc,
  };

  const safe = checkSafeSpend(payload, ctx);
  const apiCount = responseToCount(opts.apiResponse);

  let path;
  let fraudCount;
  let response;
  if (safe.pass) {
    path = "SafeFast";
    fraudCount = 0;
    response = countToResponse(0);
  } else {
    path = "Knn";
    fraudCount = apiCount ?? 3;
    response = opts.apiResponse ?? countToResponse(fraudCount);
  }

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
    checks: { safeSpend: safe },
    flowSteps,
    timingMs: { classify: performance.now() - t0 },
  };
}

function buildFlowSteps(path, api) {
  const ingress = [
    { id: "client", label: "Cliente / k6", layer: "edge" },
    { id: "lb", label: "Load Balancer :9999", layer: "infra", detail: "epoll + round-robin" },
    { id: "socket", label: "Unix socket (SCM_RIGHTS)", layer: "infra", detail: "/tmp/sockets" },
    { id: api, label: api.toUpperCase(), layer: "worker", detail: "fd_gateway" },
    { id: "http", label: "HTTP parse", layer: "handler", detail: "POST /fraud-score" },
    { id: "extract", label: "JSON extract", layer: "handler" },
    { id: "safe", label: "Gasto seguro?", layer: "scorer" },
  ];
  const tail = [
    { id: "response", label: "Resposta HTTP estatica", layer: "out" },
    { id: "client_out", label: "Cliente", layer: "edge" },
  ];

  if (path === "SafeFast") {
    return [
      ...ingress,
      { id: "approved", label: "Aprova no fast path", layer: "hit", active: true },
      ...tail,
    ];
  }

  return [
    ...ingress,
    { id: "knn", label: "k-NN exato", layer: "hit", active: true },
    ...tail,
  ];
}
