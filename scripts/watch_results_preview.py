import argparse
import hashlib
import json
import time
import urllib.request

DEFAULT_URL = (
    "https://raw.githubusercontent.com/arinhadebackend/"
    "arinhadebackend.github.io/2026-preview/results-preview.json"
)


def fetch(url: str) -> dict:
    req = urllib.request.Request(url, headers={"Cache-Control": "no-cache"})
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.load(resp)


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("username")
    p.add_argument("--url", default=DEFAULT_URL)
    p.add_argument("--interval-seconds", type=float, default=5.0)
    p.add_argument("--max-bytes", type=int, default=4000)
    args = p.parse_args()

    username: str = args.username
    interval: float = args.interval_seconds
    max_bytes: int = args.max_bytes

    last_hash: str | None = None
    print(f"SL4U_MONITOR_STARTED {username}", flush=True)

    while True:
        time.sleep(interval)
        try:
            data = fetch(args.url)
            entry = data.get(username)
            payload = json.dumps(entry, ensure_ascii=False, sort_keys=True)
            h = hashlib.sha256(payload.encode("utf-8")).hexdigest()

            if h != last_hash:
                last_hash = h
                ts = time.strftime("%Y-%m-%d %H:%M:%S")
                print(f"SL4U_MONITOR_UPDATE {ts} {payload[:max_bytes]}", flush=True)
        except Exception as e:  # noqa: BLE001 - monitor must keep running
            ts = time.strftime("%Y-%m-%d %H:%M:%S")
            msg = str(e).replace("\n", " ")
            print(f"SL4U_MONITOR_ERROR {ts} {type(e).__name__} {msg[:200]}", flush=True)


if __name__ == "__main__":
    raise SystemExit(main())
