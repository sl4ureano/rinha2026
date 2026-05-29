#!/usr/bin/env python3
"""
Port Python do data-generator oficial da Rinha 2026:
https://github.com/zanfranceschi/rinha-de-backend-2026/tree/main/data-generator

Reproduz PCG32, gen_request(), normalize() e pick_profile() com os mesmos
seeds/parâmetros do binário C (REF_SEED=42, fraud_ratio=0.30, etc.).
"""
from __future__ import annotations

import calendar
import json
import math
import pathlib
from dataclasses import dataclass, field
from datetime import datetime, timezone
from enum import Enum
from typing import Iterator

ROOT = pathlib.Path(__file__).resolve().parent.parent
RESOURCES = ROOT / "resources"

VDIM = 14
REF_SEED = 42
PAY_SEED = 4242
MAX_KNOWN = 6

LEGIT_MCCS = ("5411", "5812", "5912", "5311")
FRAUD_MCCS = ("7995", "7801", "7802")


class Profile(Enum):
    LEGIT = "legit"
    FRAUD = "fraud"
    BORDERLINE = "borderline"


@dataclass
class NormCfg:
    max_amount: float = 10_000.0
    max_installments: float = 12.0
    amount_vs_avg_ratio: float = 10.0
    max_minutes: float = 1440.0
    max_km: float = 1000.0
    max_tx_count_24h: float = 20.0
    max_merchant_avg_amount: float = 10_000.0


@dataclass
class Request:
    id: str = ""
    amount: float = 0.0
    installments: int = 0
    requested_at: str = ""
    cust_avg: float = 0.0
    tx_count_24h: int = 0
    known: list[str] = field(default_factory=list)
    merch_id: str = ""
    mcc: str = ""
    merch_avg: float = 0.0
    is_online: bool = False
    card_present: bool = False
    km_home: float = 0.0
    has_last: bool = False
    last_ts: str = ""
    last_km: float = 0.0


class Rng:
    """PCG32 — bit-a-bit compatível com main.c."""

    def __init__(self, seed: int):
        self.state = 0
        self.inc = (seed << 1) | 1
        self.pcg32()
        self.state = (self.state + seed) & 0xFFFFFFFFFFFFFFFF
        self.pcg32()

    def pcg32(self) -> int:
        s = self.state
        self.state = (s * 6364136223846793005 + self.inc) & 0xFFFFFFFFFFFFFFFF
        x = (((s >> 18) ^ s) >> 27) & 0xFFFFFFFF
        rot = (s >> 59) & 0xFFFFFFFF
        return ((x >> rot) | (x << ((-rot) & 31))) & 0xFFFFFFFF

    def f64(self) -> float:
        return self.pcg32() / 4294967295.0

    def range_f(self, lo: float, hi: float) -> float:
        return lo + self.f64() * (hi - lo)

    def range_i(self, lo: int, hi: int) -> int:
        """[lo, hi) — hi exclusivo, igual ao C."""
        return lo + (self.pcg32() % (hi - lo))


def clamp01(v: float) -> float:
    return max(0.0, min(1.0, v))


def round2(v: float) -> float:
    return round(v * 100.0) / 100.0


def round4(v: float) -> float:
    return round(v * 10000.0) / 10000.0


def day_of_week(y: int, m: int, d: int) -> int:
    """Monday=0 … Sunday=6 (Tomohiko Sakamoto, igual ao C)."""
    t = (0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4)
    if m < 3:
        y -= 1
    dow = (y + y // 4 - y // 100 + y // 400 + t[m - 1] + d) % 7
    return (dow + 6) % 7


def ts_epoch(ts: str) -> int:
    dt = datetime.strptime(ts, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    return int(dt.timestamp())


def epoch_to_ts(ep: int) -> str:
    dt = datetime.fromtimestamp(ep, tz=timezone.utc)
    return dt.strftime("%Y-%m-%dT%H:%M:%SZ")


def load_norm(path: pathlib.Path | None = None) -> NormCfg:
    path = path or RESOURCES / "normalization.json"
    with open(path, encoding="utf-8") as f:
        j = json.load(f)
    return NormCfg(
        max_amount=j["max_amount"],
        max_installments=j["max_installments"],
        amount_vs_avg_ratio=j["amount_vs_avg_ratio"],
        max_minutes=j["max_minutes"],
        max_km=j["max_km"],
        max_tx_count_24h=j["max_tx_count_24h"],
        max_merchant_avg_amount=j["max_merchant_avg_amount"],
    )


def load_mcc(path: pathlib.Path | None = None) -> dict[str, float]:
    path = path or RESOURCES / "mcc_risk.json"
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def mcc_lookup(mcc_map: dict[str, float], code: str) -> float:
    return mcc_map.get(code, 0.5)


def pick_profile(rng: Rng, fraud_ratio: float) -> Profile:
    borderline = fraud_ratio * 0.10
    v = rng.f64()
    if v < 1.0 - fraud_ratio:
        return Profile.LEGIT
    if v < 1.0 - borderline:
        return Profile.FRAUD
    return Profile.BORDERLINE


def profile_label(profile: Profile, rng: Rng) -> str:
    if profile == Profile.BORDERLINE:
        return "fraud" if rng.f64() < 0.5 else "legit"
    return profile.value


def gen_request(
    rng: Rng,
    profile: Profile,
    mcc_map: dict[str, float],
    random_dates: bool = False,
) -> Request:
    req = Request()
    req.id = f"tx-{rng.pcg32()}"

    if profile == Profile.LEGIT:
        req.amount = round2(rng.range_f(10, 500))
    elif profile == Profile.FRAUD:
        req.amount = round2(rng.range_f(2000, 10000))
    else:
        req.amount = round2(rng.range_f(400, 3000))

    if profile == Profile.LEGIT:
        req.installments = rng.range_i(1, 4)
    elif profile == Profile.FRAUD:
        req.installments = rng.range_i(6, 13)
    else:
        req.installments = rng.range_i(3, 8)

    if profile == Profile.LEGIT:
        h_lo, h_hi = 8, 21
    elif profile == Profile.FRAUD:
        h_lo, h_hi = 0, 7
    else:
        h_lo, h_hi = 6, 23
    hour = rng.range_i(h_lo, h_hi)
    minute = rng.range_i(0, 60)
    second = rng.range_i(0, 60)

    if random_dates:
        year = rng.range_i(2026, 2031)
        month = rng.range_i(3, 13) if year == 2026 else rng.range_i(1, 13)
        day = rng.range_i(1, 29)
        req.requested_at = f"{year:04d}-{month:02d}-{day:02d}T{hour:02d}:{minute:02d}:{second:02d}Z"
    else:
        day = rng.range_i(10, 28)
        req.requested_at = f"2026-03-{day:02d}T{hour:02d}:{minute:02d}:{second:02d}Z"

    if profile == Profile.LEGIT:
        req.cust_avg = round2(rng.range_f(req.amount / 0.5, req.amount * 2.0))
    elif profile == Profile.FRAUD:
        req.cust_avg = round2(rng.range_f(50, 300))
    else:
        req.cust_avg = round2(rng.range_f(100, 500))

    if profile == Profile.LEGIT:
        req.tx_count_24h = rng.range_i(1, 6)
    elif profile == Profile.FRAUD:
        req.tx_count_24h = rng.range_i(8, 21)
    else:
        req.tx_count_24h = rng.range_i(4, 12)

    known_n = rng.range_i(2, 6)
    req.known = [f"MERC-{rng.range_i(1, 20):03d}" for _ in range(known_n)]

    if profile == Profile.LEGIT:
        req.merch_id = req.known[rng.range_i(0, known_n)]
    elif profile == Profile.FRAUD:
        req.merch_id = f"MERC-{rng.range_i(50, 100):03d}"
    elif rng.f64() < 0.5:
        req.merch_id = req.known[rng.range_i(0, known_n)]
    else:
        req.merch_id = f"MERC-{rng.range_i(30, 60):03d}"

    mcc_codes = list(mcc_map.keys())
    if profile == Profile.LEGIT:
        req.mcc = LEGIT_MCCS[rng.range_i(0, 4)]
    elif profile == Profile.FRAUD:
        req.mcc = FRAUD_MCCS[rng.range_i(0, 3)]
    else:
        req.mcc = mcc_codes[rng.range_i(0, len(mcc_codes))]

    if profile == Profile.LEGIT:
        req.merch_avg = round2(rng.range_f(30, 500))
    elif profile == Profile.FRAUD:
        req.merch_avg = round2(rng.range_f(20, 100))
    else:
        req.merch_avg = round2(rng.range_f(50, 300))

    if profile == Profile.LEGIT:
        req.is_online = rng.f64() < 0.3
    elif profile == Profile.FRAUD:
        req.is_online = rng.f64() < 0.8
    else:
        req.is_online = rng.f64() < 0.5
    req.card_present = False if req.is_online else (rng.f64() < 0.9)

    if profile == Profile.LEGIT:
        req.km_home = rng.range_f(0, 50)
    elif profile == Profile.FRAUD:
        req.km_home = rng.range_f(200, 1000)
    else:
        req.km_home = rng.range_f(30, 400)

    if rng.f64() < 0.2:
        req.has_last = False
    else:
        req.has_last = True
        req_ep = ts_epoch(req.requested_at)
        if random_dates:
            r64 = (rng.pcg32() << 32) | rng.pcg32()
            secs_back = 60 + int(r64 % (10 * 365 * 24 * 3600))
        else:
            if profile == Profile.LEGIT:
                mins_back = rng.range_i(30, 720)
            elif profile == Profile.FRAUD:
                mins_back = rng.range_i(1, 10)
            else:
                mins_back = rng.range_i(5, 120)
            secs_back = mins_back * 60
        req.last_ts = epoch_to_ts(req_ep - secs_back)
        if profile == Profile.LEGIT:
            req.last_km = rng.range_f(0, 20)
        elif profile == Profile.FRAUD:
            req.last_km = rng.range_f(200, 1000)
        else:
            req.last_km = rng.range_f(20, 300)

    return req


def normalize(req: Request, cfg: NormCfg, mcc_map: dict[str, float]) -> list[float]:
    y, mo, d = (int(x) for x in req.requested_at[:10].split("-"))
    h = int(req.requested_at[11:13])
    dow = day_of_week(y, mo, d)

    out = [0.0] * VDIM
    out[0] = clamp01(req.amount / cfg.max_amount)
    out[1] = clamp01(req.installments / cfg.max_installments)
    out[2] = clamp01((req.amount / req.cust_avg) / cfg.amount_vs_avg_ratio)
    out[3] = h / 23.0
    out[4] = dow / 6.0

    if req.has_last:
        mins = (ts_epoch(req.requested_at) - ts_epoch(req.last_ts)) / 60.0
        out[5] = clamp01(mins / cfg.max_minutes)
        out[6] = clamp01(req.last_km / cfg.max_km)
    else:
        out[5] = -1.0
        out[6] = -1.0

    out[7] = clamp01(req.km_home / cfg.max_km)
    out[8] = clamp01(req.tx_count_24h / cfg.max_tx_count_24h)
    out[9] = 1.0 if req.is_online else 0.0
    out[10] = 1.0 if req.card_present else 0.0
    out[11] = 0.0 if req.merch_id in req.known else 1.0
    out[12] = mcc_lookup(mcc_map, req.mcc)
    out[13] = clamp01(req.merch_avg / cfg.max_merchant_avg_amount)
    return [round4(v) for v in out]


def request_to_dict(req: Request) -> dict:
    obj = {
        "id": req.id,
        "transaction": {
            "amount": req.amount,
            "installments": req.installments,
            "requested_at": req.requested_at,
        },
        "customer": {
            "avg_amount": req.cust_avg,
            "tx_count_24h": req.tx_count_24h,
            "known_merchants": list(req.known),
        },
        "merchant": {
            "id": req.merch_id,
            "mcc": req.mcc,
            "avg_amount": req.merch_avg,
        },
        "terminal": {
            "is_online": req.is_online,
            "card_present": req.card_present,
            "km_from_home": req.km_home,
        },
    }
    if req.has_last:
        obj["last_transaction"] = {
            "timestamp": req.last_ts,
            "km_from_current": req.last_km,
        }
    else:
        obj["last_transaction"] = None
    return obj


def generate_references(
    n: int,
    seed: int = REF_SEED,
    fraud_ratio: float = 0.30,
    norm: NormCfg | None = None,
    mcc_map: dict[str, float] | None = None,
) -> Iterator[tuple[Request, list[float], str]]:
    """Gera n referências (request bruto + vetor 14d + label)."""
    norm = norm or load_norm()
    mcc_map = mcc_map or load_mcc()
    rng = Rng(seed)
    for _ in range(n):
        profile = pick_profile(rng, fraud_ratio)
        req = gen_request(rng, profile, mcc_map, random_dates=False)
        vec = normalize(req, norm, mcc_map)
        label = profile_label(profile, rng)
        yield req, vec, label


def verify_against_references(
    path: pathlib.Path,
    n_check: int = 100,
    seed: int = REF_SEED,
    fraud_ratio: float = 0.30,
) -> bool:
    """Compara as primeiras n_check entradas geradas com references.json(.gz)."""
    import gzip

    if path.suffix == ".gz":
        with gzip.open(path, "rt", encoding="utf-8") as f:
            stored = json.load(f)
    else:
        with open(path, encoding="utf-8") as f:
            stored = json.load(f)

    ok = True
    for i, (_, vec, label) in enumerate(
        generate_references(n_check, seed=seed, fraud_ratio=fraud_ratio)
    ):
        s = stored[i]
        sv = s["vector"]
        if not all(abs(a - b) < 1e-4 for a, b in zip(vec, sv)):
            print(f"  mismatch vector[{i}]: gen={vec} stored={sv}")
            ok = False
        if label != s["label"]:
            print(f"  mismatch label[{i}]: gen={label} stored={s['label']}")
            ok = False
    return ok


if __name__ == "__main__":
    import argparse

    ap = argparse.ArgumentParser(description="Verifica ou gera dados sintéticos (port do data-generator C)")
    ap.add_argument("--verify", type=pathlib.Path, default=RESOURCES / "references.json.gz",
                    help="Compara primeiras N entradas com references existentes")
    ap.add_argument("--n-check", type=int, default=100)
    ap.add_argument("--refs", type=int, default=0, help="Gera N referências em JSON (stdout sample se 0)")
    ap.add_argument("--seed", type=int, default=REF_SEED)
    args = ap.parse_args()

    if args.verify and args.verify.exists():
        print(f"Verificando {args.n_check} entradas contra {args.verify} ...")
        if verify_against_references(args.verify, n_check=args.n_check, seed=args.seed):
            print("OK — gerador Python bate com references armazenadas.")
        else:
            raise SystemExit("FALHOU — vetores/labels divergem")
    elif args.refs > 0:
        out = []
        for _, vec, label in generate_references(args.refs, seed=args.seed):
            out.append({"vector": vec, "label": label})
        print(json.dumps(out[:3], indent=2))
        print(f"... ({args.refs} total)")
    else:
        req, vec, label = next(generate_references(1))
        print(json.dumps({"request": request_to_dict(req), "vector": vec, "label": label}, indent=2))
