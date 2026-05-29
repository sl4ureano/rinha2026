#!/usr/bin/env python3
"""
Treina a decision tree a partir de payloads brutos (ex: test/test-data.json).

Este script demonstra o pipeline COMPLETO para gerar um decision_tree.nodes
idêntico ao existente, usando dados com todas as features brutas.

Uso:
  python scripts/train_from_payloads.py <payloads.json> [--max-leaf-nodes N] [--random-state S]

O arquivo de payloads deve ter o formato de test/test-data.json:
  {
    "entries": [
      {
        "request": { ... payload completo ... },
        "expected_fraud_score": 0 ou 1
      }
    ]
  }

Para gerar o decision_tree.nodes idêntico ao existente:
  1. Usar o MESMO dataset de 3M entries usado no treinamento original
  2. Executar: python scripts/train_from_payloads.py <dataset_3M.json> --max-leaf-nodes 520 --random-state 42
  3. Rodar: python scripts/gen_decision_tree.py  (para gerar .rs e .c)
"""
from __future__ import annotations

import argparse
import json
import math
import pathlib
import re
import sys
import time
from datetime import datetime, timezone

import numpy as np

ROOT = pathlib.Path(__file__).resolve().parent.parent

MAX_AMOUNT = 10_000.0
MAX_INSTALLMENTS = 12.0
AMOUNT_VS_AVG_RATIO = 10.0
MAX_MINUTES = 1440.0
MAX_KM = 1000.0
MAX_TX_COUNT_24H = 20.0
MAX_MERCHANT_AVG = 10_000.0

MCC_RISK = {
    "5411": 0.15, "5812": 0.30, "5912": 0.20, "5944": 0.45,
    "7801": 0.80, "7802": 0.75, "7995": 0.85, "4511": 0.35,
    "5311": 0.25, "5999": 0.50,
}
DEFAULT_MCC_RISK = 0.50


def clamp01(x: float) -> float:
    return max(0.0, min(1.0, x))


def parse_iso_epoch_seconds(ts: str) -> int | None:
    """Parse ISO8601 timestamp to epoch seconds (matches tier_score.rs parse_iso)."""
    try:
        dt = datetime.fromisoformat(ts.replace("Z", "+00:00"))
        return int(dt.timestamp())
    except Exception:
        return None


def parse_iso_hour_weekday(ts: str) -> tuple[int, int] | None:
    """Returns (hour, weekday_monday0) from ISO8601 timestamp."""
    try:
        dt = datetime.fromisoformat(ts.replace("Z", "+00:00"))
        hour = dt.hour
        weekday = dt.weekday()  # Monday=0
        return hour, weekday
    except Exception:
        return None


def build_21_features(req: dict) -> list[float] | None:
    """
    Extrai as 21 features de um payload bruto, reproduzindo exatamente
    build_tree_features() de src/search/tier_score.rs.
    """
    tx = req.get("transaction", {})
    cust = req.get("customer", {})
    merch = req.get("merchant", {})
    term = req.get("terminal", {})
    last_tx = req.get("last_transaction")

    amount = float(tx.get("amount", 0))
    installments = int(tx.get("installments", 0))
    requested_at = tx.get("requested_at", "")
    customer_avg = float(cust.get("avg_amount", 1))
    tx_count_24h = int(cust.get("tx_count_24h", 0))
    known_merchants = cust.get("known_merchants", [])
    merchant_id = merch.get("id", "")
    mcc = merch.get("mcc", "")
    merchant_avg = float(merch.get("avg_amount", 0))
    is_online = bool(term.get("is_online", False))
    card_present = bool(term.get("card_present", False))
    km_from_home = float(term.get("km_from_home", 0))

    # Parse requested_at
    hw = parse_iso_hour_weekday(requested_at)
    if hw is None:
        return None
    hour, weekday = hw
    req_epoch = parse_iso_epoch_seconds(requested_at)
    if req_epoch is None:
        return None

    safe_avg = max(customer_avg, 1.0)
    amount_ratio = amount / safe_avg
    known = merchant_id in known_merchants
    mcc_risk = MCC_RISK.get(mcc, DEFAULT_MCC_RISK)

    # Last transaction features
    if last_tx is not None and last_tx.get("timestamp"):
        last_epoch = parse_iso_epoch_seconds(last_tx["timestamp"])
        if last_epoch is not None:
            delta_seconds = max(0, req_epoch - last_epoch)
            minutes_since_last = clamp01(delta_seconds / 60.0 / MAX_MINUTES)
            km_last = last_tx.get("km_from_current")
            if km_last is not None:
                km_from_last = clamp01(float(km_last) / MAX_KM)
            else:
                km_from_last = -1.0
            last_null = 0.0
        else:
            minutes_since_last = -1.0
            km_from_last = -1.0
            last_null = 1.0
    else:
        minutes_since_last = -1.0
        km_from_last = -1.0
        last_null = 1.0

    features = [
        clamp01(amount / MAX_AMOUNT),                      # 0
        clamp01(installments / MAX_INSTALLMENTS),           # 1
        clamp01(amount_ratio / AMOUNT_VS_AVG_RATIO),        # 2
        hour / 23.0,                                         # 3
        weekday / 6.0,                                       # 4
        minutes_since_last,                                   # 5
        km_from_last,                                         # 6
        clamp01(km_from_home / MAX_KM),                      # 7
        clamp01(tx_count_24h / MAX_TX_COUNT_24H),            # 8
        1.0 if is_online else 0.0,                           # 9
        1.0 if card_present else 0.0,                        # 10
        0.0 if known else 1.0,                               # 11
        mcc_risk,                                             # 12
        clamp01(merchant_avg / MAX_MERCHANT_AVG),            # 13
        last_null,                                            # 14
        amount,                                               # 15
        customer_avg,                                         # 16 ← RAW (não clampado!)
        amount_ratio,                                         # 17 ← RAW (não clampado!)
        float(tx_count_24h),                                  # 18
        km_from_home,                                         # 19
        merchant_avg,                                         # 20
    ]
    return features


def parse_existing_nodes(path: pathlib.Path):
    src = path.read_text(encoding="utf-8")
    m = re.search(r"const nodes = \[_\]Node\{(.*?)\};", src, re.S)
    if not m:
        raise SystemExit("Bloco 'const nodes' não encontrado")
    nodes = []
    for line in m.group(1).splitlines():
        line = line.strip()
        if not line.startswith(".{"):
            continue
        f = re.search(r"\.feature = (\d+|LeafFeature)", line)
        t = re.search(r"\.threshold = ([^,]+)", line)
        l = re.search(r"\.left = (-?\d+)", line)
        r_ = re.search(r"\.right = (-?\d+)", line)
        fr = re.search(r"\.fraud = (true|false)", line)
        feat = 255 if f.group(1) == "LeafFeature" else int(f.group(1))
        nodes.append({
            "feature": feat,
            "threshold": float(t.group(1)),
            "left": int(l.group(1)),
            "right": int(r_.group(1)),
            "fraud": fr.group(1) == "true",
        })
    return nodes


def sklearn_tree_to_nodes(clf):
    tree = clf.tree_
    nodes = []
    for i in range(tree.node_count):
        if tree.feature[i] < 0:
            fraud = bool(tree.value[i][0][1] > tree.value[i][0][0])
            nodes.append({
                "feature": 255, "threshold": 0.0,
                "left": -1, "right": -1, "fraud": fraud,
            })
        else:
            fraud = bool(tree.value[i][0][1] > tree.value[i][0][0])
            nodes.append({
                "feature": int(tree.feature[i]),
                "threshold": float(tree.threshold[i]),
                "left": int(tree.children_left[i]),
                "right": int(tree.children_right[i]),
                "fraud": fraud,
            })
    return nodes


def write_nodes_file(nodes, path: pathlib.Path):
    lines = [
        "// Residual decision tree node table (input for gen_decision_tree.py).",
        "pub const FeatureCount = 21;",
        "",
        "const LeafFeature: u8 = 255;",
        "",
        "const Node = struct {",
        "    feature: u8,",
        "    threshold: f32,",
        "    left: i16,",
        "    right: i16,",
        "    fraud: bool,",
        "};",
        "",
        "pub fn predict(features: *const [FeatureCount]f32) bool {",
        "    var index: usize = 0;",
        "    while (true) {",
        "        const node = nodes[index];",
        "        if (node.feature == LeafFeature) return node.fraud;",
        '        const next = if (features[@intCast(node.feature)] <= node.threshold) node.left else node.right;',
        "        index = @intCast(next);",
        "    }",
        "}",
        "",
        "const nodes = [_]Node{",
    ]
    for n in nodes:
        feat = "LeafFeature" if n["feature"] == 255 else str(n["feature"])
        th = n["threshold"]
        if th == 0:
            th_str = "0"
        elif th == int(th) and "." not in f"{th}":
            th_str = f"{int(th)}"
        else:
            th_str = f"{th}"
        fraud = "true" if n["fraud"] else "false"
        lines.append(
            f"    .{{ .feature = {feat}, .threshold = {th_str}, "
            f".left = {n['left']}, .right = {n['right']}, .fraud = {fraud} }},"
        )
    lines.append("};")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"Escrito: {path}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("data_file", type=pathlib.Path,
                     help="Arquivo JSON com payloads brutos (formato test-data.json)")
    ap.add_argument("--max-leaf-nodes", type=int, default=520,
                     help="max_leaf_nodes do DecisionTreeClassifier (default: 520 → 1039 nós)")
    ap.add_argument("--random-state", type=int, default=42,
                     help="random_state do DecisionTreeClassifier (default: 42)")
    ap.add_argument("--output", type=pathlib.Path, default=None,
                     help="Caminho de saída do .nodes (default: scripts/decision_tree.nodes)")
    args = ap.parse_args()

    try:
        from sklearn.tree import DecisionTreeClassifier
    except ImportError:
        import subprocess
        subprocess.check_call([sys.executable, "-m", "pip", "install", "scikit-learn"])
        from sklearn.tree import DecisionTreeClassifier

    # 1. Carregar payloads
    print(f"Carregando {args.data_file} ...", flush=True)
    t0 = time.time()
    with open(args.data_file) as f:
        raw = json.load(f)
    entries = raw.get("entries", raw) if isinstance(raw, dict) else raw
    print(f"  {len(entries)} entries em {time.time()-t0:.1f}s")

    # 2. Extrair 21 features
    print("Extraindo 21 features dos payloads ...", flush=True)
    features_list = []
    labels_list = []
    skipped = 0
    for entry in entries:
        req = entry.get("request", entry)
        feats = build_21_features(req)
        if feats is None:
            skipped += 1
            continue
        features_list.append(feats)
        # Label: expected_fraud_score, ou "label"
        label = entry.get("expected_fraud_score", None)
        if label is None:
            label = 1 if entry.get("label", "legit") == "fraud" else 0
        labels_list.append(int(label))

    X = np.array(features_list, dtype=np.float32)
    y = np.array(labels_list, dtype=np.int8)
    print(f"  Features: {X.shape}")
    print(f"  Skipped: {skipped}")
    print(f"  Legit: {int((y==0).sum())}  Fraud: {int((y==1).sum())}")

    # Verificar feature 17 (ratio) para entradas com ratio > 10
    high_ratio = (X[:, 17] > 10).sum()
    print(f"  Entries com ratio > 10 (feature 17): {int(high_ratio)}")
    print(f"  Entries com ratio > 35 (feature 17): {int((X[:, 17] > 35).sum())}")

    # 3. Treinar
    print(f"\nTreinando: max_leaf_nodes={args.max_leaf_nodes}, random_state={args.random_state}")
    clf = DecisionTreeClassifier(
        criterion="gini",
        max_leaf_nodes=args.max_leaf_nodes,
        random_state=args.random_state,
    )
    clf.fit(X, y)
    gen_nodes = sklearn_tree_to_nodes(clf)
    print(f"Nós gerados: {len(gen_nodes)}")

    # Features usadas
    used = set(n["feature"] for n in gen_nodes if n["feature"] != 255)
    print(f"Features usadas: {sorted(used)}")

    # 4. Comparar com existente se disponível
    existing_path = ROOT / "scripts" / "decision_tree.nodes"
    if existing_path.exists():
        existing_nodes = parse_existing_nodes(existing_path)
        print(f"\nComparação com árvore existente ({len(existing_nodes)} nós):")
        if len(existing_nodes) == len(gen_nodes):
            feat_match = sum(1 for e, g in zip(existing_nodes, gen_nodes) if e["feature"] == g["feature"])
            print(f"  Feature match: {feat_match}/{len(existing_nodes)} ({100*feat_match/len(existing_nodes):.1f}%)")
        else:
            print(f"  Quantidade de nós diferente: {len(existing_nodes)} vs {len(gen_nodes)}")

    # 5. Salvar
    out_path = args.output or ROOT / "scripts" / "decision_tree.nodes"
    write_nodes_file(gen_nodes, out_path)

    print(f"\nPróximo passo: python scripts/gen_decision_tree.py")
    print("  → gera src/search/decision_tree.rs e arquivos C")


if __name__ == "__main__":
    main()
