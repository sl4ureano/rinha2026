#!/usr/bin/env python3
import datetime as dt
import html
import json
import os
import sqlite3
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


DB_PATH = os.environ.get("WEBHOOK_DB", "webhook_requests.sqlite3")
BIND = os.environ.get("WEBHOOK_BIND", "0.0.0.0")
PORT = int(os.environ.get("WEBHOOK_PORT", "8080"))
TRUST_PROXY = os.environ.get("WEBHOOK_TRUST_PROXY", "").lower() in {"1", "true", "yes"}
MAX_BODY_BYTES = int(os.environ.get("WEBHOOK_MAX_BODY_BYTES", str(10 * 1024 * 1024)))
CAPTURE_GET = os.environ.get("WEBHOOK_CAPTURE_GET", "").lower() in {"1", "true", "yes"}
BODY_PREVIEW_BYTES = 64 * 1024

DB_LOCK = threading.Lock()


def init_db():
    with sqlite3.connect(DB_PATH) as con:
        con.execute(
            """
            create table if not exists requests (
                id integer primary key autoincrement,
                received_at text not null,
                ip text not null,
                method text not null,
                path text not null,
                query text not null,
                headers_json text not null,
                content_type text not null,
                body blob not null,
                body_size integer not null
            )
            """
        )
        con.execute("create index if not exists idx_requests_received_at on requests(received_at)")
        con.commit()


def db_execute(sql, args=(), one=False):
    with DB_LOCK:
        with sqlite3.connect(DB_PATH) as con:
            con.row_factory = sqlite3.Row
            cur = con.execute(sql, args)
            rows = cur.fetchall()
            con.commit()
            if one:
                return rows[0] if rows else None
            return rows


def client_ip(handler):
    if TRUST_PROXY:
        forwarded = handler.headers.get("x-forwarded-for", "")
        if forwarded:
            return forwarded.split(",", 1)[0].strip()
        real_ip = handler.headers.get("x-real-ip", "")
        if real_ip:
            return real_ip.strip()
    return handler.client_address[0]


def utc_now():
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="milliseconds")


def decode_body(body, content_type):
    text = body[:BODY_PREVIEW_BYTES].decode("utf-8", errors="replace")
    if "application/json" in content_type.lower():
        try:
            return json.dumps(json.loads(text), indent=2, ensure_ascii=False)
        except Exception:
            return text
    return text


def row_to_dict(row, include_body=False):
    item = {
        "id": row["id"],
        "received_at": row["received_at"],
        "ip": row["ip"],
        "method": row["method"],
        "path": row["path"],
        "query": row["query"],
        "content_type": row["content_type"],
        "body_size": row["body_size"],
        "headers": json.loads(row["headers_json"]),
    }
    if include_body:
        item["body"] = decode_body(row["body"], row["content_type"])
    return item


class Handler(BaseHTTPRequestHandler):
    server_version = "VpsWebhook/1.0"

    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)
        if parsed.path == "/":
            return self.dashboard()
        if parsed.path == "/favicon.ico":
            return self.text(204, "")
        if parsed.path == "/api/requests":
            return self.api_requests(parsed.query)
        if parsed.path.startswith("/api/requests/"):
            return self.api_request(parsed.path.rsplit("/", 1)[-1])
        if parsed.path.startswith("/request/"):
            return self.request_page(parsed.path.rsplit("/", 1)[-1])
        if parsed.path.startswith("/body/"):
            return self.raw_body(parsed.path.rsplit("/", 1)[-1])
        if parsed.path == "/health":
            return self.text(200, "ok\n")
        if CAPTURE_GET:
            return self.capture()
        return self.text(404, "not found\n")

    def do_HEAD(self):
        self.send_response(204)
        self.end_headers()

    def do_OPTIONS(self):
        self.send_response(204)
        self.send_header("Allow", "GET, HEAD, OPTIONS, POST, PUT, PATCH, DELETE")
        self.end_headers()

    def do_POST(self):
        return self.capture()

    def do_PUT(self):
        return self.capture()

    def do_PATCH(self):
        return self.capture()

    def do_DELETE(self):
        return self.capture()

    def capture(self):
        length = int(self.headers.get("content-length", "0") or "0")
        if length > MAX_BODY_BYTES:
            self.send_response(413)
            self.end_headers()
            return

        body = self.rfile.read(length) if length else b""
        parsed = urllib.parse.urlparse(self.path)
        headers = {k.lower(): v for k, v in self.headers.items()}
        content_type = self.headers.get("content-type", "")
        now = utc_now()
        ip = client_ip(self)

        with DB_LOCK:
            with sqlite3.connect(DB_PATH) as con:
                cur = con.execute(
                    """
                    insert into requests
                    (received_at, ip, method, path, query, headers_json, content_type, body, body_size)
                    values (?, ?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        now,
                        ip,
                        self.command,
                        parsed.path,
                        parsed.query,
                        json.dumps(headers, ensure_ascii=False),
                        content_type,
                        body,
                        len(body),
                    ),
                )
                request_id = cur.lastrowid
                con.commit()

        self.json_response(
            200,
            {
                "ok": True,
                "id": request_id,
                "received_at": now,
                "ip": ip,
                "body_size": len(body),
            },
        )

    def dashboard(self):
        rows = db_execute(
            """
            select id, received_at, ip, method, path, query, content_type, body_size
            from requests
            order by id desc
            limit 200
            """
        )
        total = db_execute("select count(*) as n from requests", one=True)["n"]
        last = rows[0]["received_at"] if rows else "-"
        items = "\n".join(
            f"""
            <tr>
              <td><a href="/request/{row['id']}">#{row['id']}</a></td>
              <td>{esc(row['received_at'])}</td>
              <td>{esc(row['ip'])}</td>
              <td>{esc(row['method'])}</td>
              <td>{esc(row['path'])}{'?' + esc(row['query']) if row['query'] else ''}</td>
              <td>{esc(row['content_type'])}</td>
              <td>{row['body_size']}</td>
            </tr>
            """
            for row in rows
        )
        return self.html(
            200,
            f"""
            <!doctype html>
            <html>
            <head>
              <meta charset="utf-8">
              <meta name="viewport" content="width=device-width, initial-scale=1">
              <meta http-equiv="refresh" content="10">
              <title>VPS Webhook</title>
              <style>{CSS}</style>
            </head>
            <body>
              <main>
                <h1>VPS Webhook</h1>
                <section class="cards">
                  <div><b>{total}</b><span>requests salvas</span></div>
                  <div><b>{esc(last)}</b><span>última request</span></div>
                  <div><b>POST /qualquer-coisa</b><span>endpoint de captura</span></div>
                </section>
                <p class="hint">Use como URL: <code>http://SEU_IP:{PORT}/perf</code>. A página atualiza a cada 10s.</p>
                <table>
                  <thead>
                    <tr>
                      <th>ID</th><th>Data UTC</th><th>IP</th><th>Método</th><th>Path</th><th>Content-Type</th><th>Bytes</th>
                    </tr>
                  </thead>
                  <tbody>{items}</tbody>
                </table>
              </main>
            </body>
            </html>
            """,
        )

    def request_page(self, request_id):
        row = load_request(request_id)
        if not row:
            return self.text(404, "request not found\n")
        body = decode_body(row["body"], row["content_type"])
        headers = json.dumps(json.loads(row["headers_json"]), indent=2, ensure_ascii=False)
        return self.html(
            200,
            f"""
            <!doctype html>
            <html>
            <head>
              <meta charset="utf-8">
              <meta name="viewport" content="width=device-width, initial-scale=1">
              <title>Request #{row['id']}</title>
              <style>{CSS}</style>
            </head>
            <body>
              <main>
                <p><a href="/">← voltar</a></p>
                <h1>Request #{row['id']}</h1>
                <section class="cards">
                  <div><b>{esc(row['received_at'])}</b><span>data UTC</span></div>
                  <div><b>{esc(row['ip'])}</b><span>IP</span></div>
                  <div><b>{row['body_size']}</b><span>bytes</span></div>
                </section>
                <h2>{esc(row['method'])} {esc(row['path'])}{'?' + esc(row['query']) if row['query'] else ''}</h2>
                <p><a href="/body/{row['id']}">baixar body bruto</a></p>
                <h3>Headers</h3>
                <pre>{esc(headers)}</pre>
                <h3>Body</h3>
                <pre>{esc(body)}</pre>
              </main>
            </body>
            </html>
            """,
        )

    def api_requests(self, query):
        params = urllib.parse.parse_qs(query)
        limit = min(int(params.get("limit", ["100"])[0]), 1000)
        rows = db_execute(
            """
            select id, received_at, ip, method, path, query, headers_json, content_type, body, body_size
            from requests
            order by id desc
            limit ?
            """,
            (limit,),
        )
        return self.json_response(200, [row_to_dict(row, include_body=False) for row in rows])

    def api_request(self, request_id):
        row = load_request(request_id)
        if not row:
            return self.json_response(404, {"error": "request not found"})
        return self.json_response(200, row_to_dict(row, include_body=True))

    def raw_body(self, request_id):
        row = load_request(request_id)
        if not row:
            return self.text(404, "request not found\n")
        self.send_response(200)
        self.send_header("Content-Type", row["content_type"] or "application/octet-stream")
        self.send_header("Content-Length", str(len(row["body"])))
        self.end_headers()
        self.wfile.write(row["body"])

    def html(self, status, body):
        data = body.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def text(self, status, body):
        data = body.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def json_response(self, status, payload):
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, fmt, *args):
        print(f"{utc_now()} {self.client_address[0]} {fmt % args}", flush=True)


def load_request(request_id):
    try:
        rid = int(request_id)
    except ValueError:
        return None
    return db_execute(
        """
        select id, received_at, ip, method, path, query, headers_json, content_type, body, body_size
        from requests
        where id = ?
        """,
        (rid,),
        one=True,
    )


def esc(value):
    return html.escape(str(value), quote=True)


CSS = """
:root { color-scheme: dark; }
body { margin: 0; background: #0f172a; color: #e5e7eb; font: 14px/1.45 system-ui, sans-serif; }
main { max-width: 1200px; margin: 0 auto; padding: 28px; }
a { color: #93c5fd; text-decoration: none; }
a:hover { text-decoration: underline; }
h1 { margin: 0 0 18px; font-size: 28px; }
h2, h3 { margin-top: 24px; }
code, pre { background: #111827; border: 1px solid #1f2937; border-radius: 8px; }
code { padding: 2px 6px; }
pre { padding: 14px; overflow: auto; white-space: pre-wrap; }
.hint { color: #cbd5e1; }
.cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px; margin: 18px 0; }
.cards div { background: #111827; border: 1px solid #1f2937; border-radius: 12px; padding: 14px; }
.cards b { display: block; font-size: 18px; margin-bottom: 6px; word-break: break-all; }
.cards span { color: #94a3b8; }
table { width: 100%; border-collapse: collapse; margin-top: 18px; background: #111827; border-radius: 12px; overflow: hidden; }
th, td { border-bottom: 1px solid #1f2937; padding: 10px; text-align: left; vertical-align: top; }
th { color: #cbd5e1; background: #0b1220; }
td { word-break: break-word; }
"""


if __name__ == "__main__":
    init_db()
    print(f"listening on http://{BIND}:{PORT} db={DB_PATH}", flush=True)
    ThreadingHTTPServer((BIND, PORT), Handler).serve_forever()
