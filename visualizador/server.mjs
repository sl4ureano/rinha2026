import http from "node:http";
import { readFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, extname } from "node:path";
import { loadDecisionTree } from "./lib/tree-loader.mjs";
import { traceRequest } from "./lib/tier-engine.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const PUBLIC = join(__dirname, "public");
const EXAMPLES = join(__dirname, "..", "resources", "example-payloads.json");

const PORT = Number(process.env.VIZ_PORT || 3333);
const FRAUD_API = process.env.FRAUD_API_URL || "http://127.0.0.1:9999";

const treeNodes = loadDecisionTree();
let rr = 0;

/** @type {Set<import('node:http').ServerResponse>} */
const sseClients = new Set();

function broadcast(event) {
  const data = `data: ${JSON.stringify(event)}\n\n`;
  for (const res of sseClients) {
    res.write(data);
  }
}

function mime(path) {
  const ext = extname(path);
  return (
    {
      ".html": "text/html; charset=utf-8",
      ".css": "text/css; charset=utf-8",
      ".js": "application/javascript; charset=utf-8",
      ".json": "application/json",
    }[ext] || "application/octet-stream"
  );
}

function serveStatic(req, res) {
  let path = req.url?.split("?")[0] || "/";
  if (path === "/") path = "/index.html";
  const file = join(PUBLIC, path);
  if (!file.startsWith(PUBLIC) || !existsSync(file)) {
    res.writeHead(404);
    res.end("Not found");
    return;
  }
  res.writeHead(200, { "Content-Type": mime(file) });
  res.end(readFileSync(file));
}

async function proxyFraudScore(body) {
  const t0 = performance.now();
  try {
    const r = await fetch(`${FRAUD_API}/fraud-score`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    const text = await r.text();
    let json = null;
    try {
      json = JSON.parse(text);
    } catch {
      json = { raw: text };
    }
    return {
      ok: r.ok,
      status: r.status,
      body: json,
      timingMs: performance.now() - t0,
    };
  } catch (err) {
    return {
      ok: false,
      status: 0,
      error: String(err.message || err),
      timingMs: performance.now() - t0,
    };
  }
}

function readJson(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}"));
      } catch (e) {
        reject(e);
      }
    });
    req.on("error", reject);
  });
}

const server = http.createServer(async (req, res) => {
  const url = req.url?.split("?")[0] || "/";

  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
  res.setHeader("Access-Control-Allow-Headers", "Content-Type");

  if (req.method === "OPTIONS") {
    res.writeHead(204);
    res.end();
    return;
  }

  if (url === "/api/health" && req.method === "GET") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(
      JSON.stringify({
        ok: true,
        fraudApi: FRAUD_API,
        treeNodes: treeNodes.length,
      }),
    );
    return;
  }

  if (url === "/api/events" && req.method === "GET") {
    res.writeHead(200, {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache",
      Connection: "keep-alive",
    });
    res.write(`data: ${JSON.stringify({ type: "hello", fraudApi: FRAUD_API, treeNodes: treeNodes.length })}\n\n`);
    sseClients.add(res);
    req.on("close", () => sseClients.delete(res));
    return;
  }

  if (url === "/api/examples" && req.method === "GET") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(readFileSync(EXAMPLES, "utf8"));
    return;
  }

  if (url === "/api/trace" && req.method === "POST") {
    try {
      const body = await readJson(req);
      const api = rr++ % 2 === 0 ? "api1" : "api2";
      const trace = traceRequest(body, treeNodes, { api });
      const proxy = await proxyFraudScore(body);
      const event = {
        type: "flow",
        at: Date.now(),
        trace,
        proxy,
        match:
          trace.ok &&
          proxy.ok &&
          proxy.body?.approved === trace.response?.approved &&
          proxy.body?.fraud_score === trace.response?.fraud_score,
      };
      broadcast(event);
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify(event));
    } catch (e) {
      res.writeHead(400, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: String(e.message || e) }));
    }
    return;
  }

  if (url === "/api/simulate" && req.method === "POST") {
    try {
      const body = await readJson(req);
      const api = rr++ % 2 === 0 ? "api1" : "api2";
      const trace = traceRequest(body, treeNodes, { api });
      const event = { type: "flow", at: Date.now(), trace, proxy: null, match: null };
      broadcast(event);
      res.writeHead(200, { "Content-Type": "application/json" });
      res.end(JSON.stringify(event));
    } catch (e) {
      res.writeHead(400, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: String(e.message || e) }));
    }
    return;
  }

  if (req.method === "GET" && !url.startsWith("/api")) {
    serveStatic(req, res);
    return;
  }

  res.writeHead(404);
  res.end("Not found");
});

server.listen(PORT, () => {
  console.log(`\n  🎮 Rinha Flow Visualizador`);
  console.log(`  → http://localhost:${PORT}`);
  console.log(`  → API fraude: ${FRAUD_API}`);
  console.log(`  → Árvore: ${treeNodes.length} nós\n`);
});
