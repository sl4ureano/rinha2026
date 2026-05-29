#!/usr/bin/env python3
"""
Treina uma DecisionTreeClassifier a partir de resources/references.json.gz
para gerar um decision_tree.nodes.

=== Pipeline de treinamento original ===

1. Dados de transações sintéticas (3M entradas) são gerados com campos brutos:
   amount, installments, hour, weekday, minutes_since_last, km_from_last,
   km_from_home, tx_count_24h, is_online, card_present, unknown_merchant,
   mcc_risk, merchant_avg_amount, customer_avg_amount, amount_ratio, etc.

2. São computadas 21 features para treinar a árvore (build_tree_features em tier_score.rs):
   [0]  clamp01(amount / 10000)
   [1]  clamp01(installments / 12)
   [2]  clamp01(amount_ratio / 10)
   [3]  hour / 23
   [4]  weekday / 6
   [5]  minutes_since_last / 1440 (ou -1 se null)
   [6]  km_from_last / 1000 (ou -1 se null)
   [7]  clamp01(km_from_home / 1000)
   [8]  clamp01(tx_count_24h / 20)
   [9]  is_online (0/1)
   [10] card_present (0/1)
   [11] unknown_merchant (0/1)
   [12] mcc_risk (float do mcc_risk.json)
   [13] clamp01(merchant_avg_amount / 10000)
   [14] last_null (1 se sem last_transaction)
   [15] amount (bruto, sem clamp)
   [16] customer_avg_amount (bruto)
   [17] amount_ratio (bruto, sem clamp)
   [18] tx_count_24h (inteiro bruto)
   [19] km_from_home (bruto)
   [20] merchant_avg_amount (bruto)

3. São computadas 14 features normalizadas (features 0-13) → salvas em references.json.gz
   (para o índice k-NN vetorial).

4. Treinamento com sklearn:
     DecisionTreeClassifier(criterion='gini', max_leaf_nodes=520, random_state=42)
   Isso produz exatamente 1039 nós (520 folhas + 519 nós internos).

5. Exportação → scripts/decision_tree.nodes → gen_decision_tree.py → .rs e .c

=== Problema de reprodução a partir de references.json.gz ===

O references.json.gz armazena APENAS os 14 features normalizados (dims 0-13).
Features 14-20 são valores brutos que podem ser derivados dos normalizados,
EXCETO quando há clamp01:
  - feature[2] = clamp01(ratio/10): quando ratio >= 10, v[2]=1.0 e o ratio exato se perde
  - feature[0] = clamp01(amount/10000): quando amount >= 10000, v[0]=1.0

~943k entries (31%) têm v[2]=1.0 (ratio clampado).
A árvore usa features 16 e 17 em 34 splits, com thresholds como:
  - feature 16 (avg): 103.6, 142.9, 189.5, 247.0, 369.0, 424.9, etc.
  - feature 17 (ratio): 10.6, 11.2, 15.3, 16.1, 35.2

Sem os valores brutos originais, NÃO é possível gerar uma árvore idêntica.
Com a aproximação ratio=v[2]*10, o máximo ratio seria 10.0, insuficiente
para os splits em 11.2, 15.3, 35.2 etc.

=== O que este script faz ===

Demonstra o pipeline de treinamento usando a melhor aproximação possível
dos dados em resources/. Gera uma árvore com mesma quantidade de nós (1039)
mas com estrutura de splits diferente nas regiões que dependem de features 16/17.

Para reprodução EXATA, é necessário:
  - Os payloads brutos originais (com amount_ratio e customer_avg reais)
  - OU o gerador oficial com o mesmo seed:
    https://github.com/zanfranceschi/rinha-de-backend-2026/tree/main/data-generator
    (--refs 3000000 --refs-seed 42 --fraud-ratio-refs 0.30)

Uso recomendado (features brutas 16/17 corretas):
  python scripts/train_decision_tree.py --from-generator 3000000
  python scripts/gen_decision_tree.py
"""
from __future__ import annotations

import argparse
import gzip
import json
import pathlib
import re
import sys
import time

import numpy as np

ROOT = pathlib.Path(__file__).resolve().parent.parent
RESOURCES = ROOT / "resources"

MAX_AMOUNT = 10_000.0
MAX_INSTALLMENTS = 12.0
AMOUNT_VS_AVG_RATIO = 10.0
MAX_MINUTES = 1440.0
MAX_KM = 1000.0
MAX_TX_COUNT_24H = 20.0
MAX_MERCHANT_AVG = 10_000.0


def load_references(path: pathlib.Path):
    print(f"Carregando {path} ...", flush=True)
    t0 = time.time()
    with gzip.open(path, "rt", encoding="utf-8") as f:
        data = json.load(f)
    vecs = np.array([d["vector"] for d in data], dtype=np.float32)
    labels = np.array([1 if d["label"] == "fraud" else 0 for d in data], dtype=np.int8)
    print(f"  {len(data)} entries em {time.time()-t0:.1f}s")
    print(f"  legit={int((labels==0).sum())}  fraud={int((labels==1).sum())}")
    return vecs, labels


def expand_to_21(v14: np.ndarray) -> np.ndarray:
    """Expande (N,14) → (N,21) derivando features brutas das normalizadas."""
    n = v14.shape[0]
    v21 = np.zeros((n, 21), dtype=np.float32)
    v21[:, :14] = v14

    # f14: last_null
    v21[:, 14] = np.where(v14[:, 5] == -1.0, 1.0, 0.0)

    # f15: raw amount
    v21[:, 15] = v14[:, 0] * MAX_AMOUNT

    # f17: raw amount_ratio (= v[2] * 10, APROXIMAÇÃO para clampados)
    v21[:, 17] = v14[:, 2] * AMOUNT_VS_AVG_RATIO

    # f16: raw customer_avg_amount (= amount / ratio)
    ratio = v21[:, 17].copy()
    ratio[ratio <= 0] = 1e-6
    v21[:, 16] = v21[:, 15] / ratio

    # f18: raw tx_count_24h
    v21[:, 18] = np.round(v14[:, 8] * MAX_TX_COUNT_24H)

    # f19: raw km_from_home
    v21[:, 19] = v14[:, 7] * MAX_KM

    # f20: raw merchant_avg_amount
    v21[:, 20] = v14[:, 13] * MAX_MERCHANT_AVG

    return v21


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
        r = re.search(r"\.right = (-?\d+)", line)
        fr = re.search(r"\.fraud = (true|false)", line)
        feat = 255 if f.group(1) == "LeafFeature" else int(f.group(1))
        nodes.append({
            "feature": feat,
            "threshold": float(t.group(1)),
            "left": int(l.group(1)),
            "right": int(r.group(1)),
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


def compare_nodes(existing, generated):
    if len(existing) != len(generated):
        print(f"Quantidade de nós diferente: existente={len(existing)}, gerado={len(generated)}")
        return False

    feat_match = 0
    th_match = 0
    total_internal = 0
    for i, (e, g) in enumerate(zip(existing, generated)):
        if e["feature"] == g["feature"]:
            feat_match += 1
            if e["feature"] != 255:
                total_internal += 1
                if abs(e["threshold"] - g["threshold"]) < 1e-4:
                    th_match += 1
        elif e["feature"] != 255:
            total_internal += 1

    total_leaf = len(existing) - total_internal
    print(f"  Feature match: {feat_match}/{len(existing)} ({100*feat_match/len(existing):.1f}%)")
    if total_internal > 0:
        print(f"  Threshold match (internal): {th_match}/{total_internal}")
    return feat_match == len(existing) and th_match == total_internal


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


def load_from_generator(n: int, seed: int, fraud_ratio: float):
    """Gera n transações sintéticas (port do data-generator C) com payloads brutos."""
    from synthetic_data import generate_references, request_to_dict
    from train_from_payloads import build_21_features

    print(f"\nGerando {n} transações sintéticas (seed={seed}, fraud_ratio={fraud_ratio}) ...")
    t0 = time.time()
    features_list = []
    labels_list = []
    skipped = 0
    high_ratio = 0
    for req, _vec14, label in generate_references(n, seed=seed, fraud_ratio=fraud_ratio):
        feats = build_21_features(request_to_dict(req))
        if feats is None:
            skipped += 1
            continue
        features_list.append(feats)
        labels_list.append(1 if label == "fraud" else 0)
        if feats[17] > 10:
            high_ratio += 1
    X = np.array(features_list, dtype=np.float32)
    y = np.array(labels_list, dtype=np.int8)
    print(f"  {X.shape[0]} entries em {time.time()-t0:.1f}s (skipped={skipped})")
    print(f"  legit={int((y==0).sum())}  fraud={int((y==1).sum())}")
    print(f"  Entries com ratio > 10 (feature 17): {high_ratio}")
    print(f"  Entries com ratio > 35 (feature 17): {int((X[:, 17] > 35).sum())}")
    return X, y


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument(
        "--from-generator", type=int, metavar="N", default=0,
        help="Gera N payloads via synthetic_data.py (recomendado: 3000000, seed 42)",
    )
    ap.add_argument("--seed", type=int, default=42, help="RNG seed (default: 42 = REF_SEED do C)")
    ap.add_argument("--fraud-ratio", type=float, default=0.30, help="Fração de fraud (default: 0.30)")
    ap.add_argument("--max-leaf-nodes", type=int, default=520)
    ap.add_argument("--random-state", type=int, default=42)
    args = ap.parse_args()

    try:
        from sklearn.tree import DecisionTreeClassifier
    except ImportError:
        import subprocess
        subprocess.check_call([sys.executable, "-m", "pip", "install", "scikit-learn"])
        from sklearn.tree import DecisionTreeClassifier

    # 1. Carregar ou gerar dados
    if args.from_generator > 0:
        X, y = load_from_generator(args.from_generator, args.seed, args.fraud_ratio)
    else:
        v14, labels = load_references(RESOURCES / "references.json.gz")
        print("\nExpandindo 14 → 21 features (aproximação — ratio clampado se perde) ...")
        X = expand_to_21(v14)
        y = labels
        print(f"  Shape: {X.shape}")
        print(f"  Entries com v[2]=1.0 (ratio clampado): {int((v14[:, 2] == 1.0).sum())}")
        print(f"  Entries com v[0]=1.0 (amount clampado): {int((v14[:, 0] == 1.0).sum())}")
        print("\n  Dica: use --from-generator 3000000 para features brutas 16/17 corretas.")

    # 3. Carregar árvore existente
    existing_nodes = parse_existing_nodes(ROOT / "scripts" / "decision_tree.nodes")
    print(f"\nÁrvore existente: {len(existing_nodes)} nós")
    used = set(n["feature"] for n in existing_nodes if n["feature"] != 255)
    print(f"Features usadas: {sorted(used)}")

    # 4. Treinar com parâmetros conhecidos
    print(f"\n=== Treinando: gini, max_leaf_nodes={args.max_leaf_nodes}, random_state={args.random_state} ===")
    clf = DecisionTreeClassifier(
        criterion="gini",
        max_leaf_nodes=args.max_leaf_nodes,
        random_state=args.random_state,
    )
    clf.fit(X, y)
    gen_nodes = sklearn_tree_to_nodes(clf)
    print(f"Nós gerados: {len(gen_nodes)}")

    print("\nComparação com árvore existente:")
    match = compare_nodes(existing_nodes, gen_nodes)

    if match:
        print("\nOK - Arvores IDENTICAS!")
    else:
        print("\nX Arvores DIFERENTES")
        if args.from_generator <= 0:
            print("  (esperado com features 16/17 aproximadas a partir de references.json.gz)")
        print("\nPara reprodução idêntica, é necessário:")
        print("  1. Os payloads brutos originais (com customer_avg_amount e amount_ratio reais)")
        print("  2. OU o gerador oficial: --from-generator 3000000 --seed 42")
        print(f"  3. Hiperparâmetros: max_leaf_nodes={args.max_leaf_nodes}, random_state={args.random_state}")

    # Salvar a versão gerada para comparação
    out_path = ROOT / "scripts" / "decision_tree_generated.nodes"
    write_nodes_file(gen_nodes, out_path)

    # Estatísticas
    gen_used = set(n["feature"] for n in gen_nodes if n["feature"] != 255)
    print(f"\nFeatures na árvore gerada: {sorted(gen_used)}")
    print(f"Features na árvore existente: {sorted(used)}")


if __name__ == "__main__":
    main()
