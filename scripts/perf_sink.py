from http.server import BaseHTTPRequestHandler, HTTPServer


OUT = "/out/perf-snapshots.jsonl"


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", "0"))
        body = self.rfile.read(length)
        with open(OUT, "ab") as f:
            f.write(body.replace(b"\n", b" "))
            f.write(b"\n")
        self.send_response(204)
        self.end_headers()

    def log_message(self, *_args):
        return


if __name__ == "__main__":
    open(OUT, "wb").close()
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
