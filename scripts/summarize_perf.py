import json
from pathlib import Path


snapshots = [
    json.loads(line)
    for line in Path("test/perf-snapshots.jsonl").read_text().splitlines()
    if line.strip()
]
last = snapshots[-1]
results = json.loads(Path("test/results.json").read_text())

stages = {}
cache_hits = 0
cache_misses = 0
requests = 0
bytes_in = 0
bytes_out = 0
roles = []
cpu = []
rss = []
sockets = {}

for proc in last["relayed"]:
    roles.append(proc["role"])
    requests += proc["requests"]
    cache_hits += proc["cache_hits"]
    cache_misses += proc["cache_misses"]
    bytes_in += proc["bytes_received"]
    bytes_out += proc["bytes_sent"]
    cpu.append((proc["role"], proc["cpu"]["percent"]))
    rss.append((proc["role"], proc["memory"]["rss_mb"]))
    sockets[proc["role"]] = proc["socket"]
    for stage in proc["stages_cumulative"]:
        entry = stages.setdefault(
            stage["stage"],
            {"count": 0, "total_us": 0, "p99": 0, "p999": 0, "max": 0},
        )
        entry["count"] += stage["count"]
        entry["total_us"] += stage["total_us"]
        entry["p99"] = max(entry["p99"], stage["p99_us"])
        entry["p999"] = max(entry["p999"], stage["p999_us"])
        entry["max"] = max(entry["max"], stage["max_us"])

print("snapshots", len(snapshots))
print("k6_p99", results["p99"])
print("score", results["scoring"]["final_score"])
print("lb", last["local"]["lb"])
print("roles", roles)
print("requests", requests)
print("cache_hit_rate", round(cache_hits * 100 / (cache_hits + cache_misses), 3))
print("bytes_in", bytes_in)
print("bytes_out", bytes_out)
print("cpu", cpu)
print("rss", rss)
print("sockets", sockets)
for name, data in sorted(stages.items(), key=lambda item: item[1]["total_us"], reverse=True):
    if data["count"] == 0:
        continue
    print(
        name,
        "count",
        data["count"],
        "avg",
        round(data["total_us"] / data["count"], 2),
        "p99max",
        data["p99"],
        "p999max",
        data["p999"],
        "max",
        data["max"],
        "total_us",
        data["total_us"],
    )
