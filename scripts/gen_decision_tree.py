#!/usr/bin/env python3
"""Gera decision_tree (.rs e .c/.h) a partir de scripts/decision_tree.nodes."""
from __future__ import annotations

import argparse
import os
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent


def parse_nodes(src: str) -> list[tuple[int, str, str, int, int, bool]]:
    m = re.search(r"const nodes = \[_\]Node\{(.*?)\};", src, re.S)
    if not m:
        raise SystemExit("decision_tree.nodes: bloco const nodes não encontrado")
    body = m.group(1)
    nodes: list[tuple[int, str, str, int, int, bool]] = []
    for line in body.splitlines():
        line = line.strip()
        if not line.startswith(".{"):
            continue
        f = re.search(r"\.feature = (\d+|LeafFeature)", line)
        t = re.search(r"\.threshold = ([^,]+)", line)
        l = re.search(r"\.left = (-?\d+)", line)
        r = re.search(r"\.right = (-?\d+)", line)
        fr = re.search(r"\.fraud = (true|false)", line)
        feat = 255 if f.group(1) == "LeafFeature" else int(f.group(1))
        th = t.group(1)
        if th == "0":
            th_rust, th_c = "0.0", "0.f"
        elif "." not in th:
            th_rust, th_c = th + ".0", th + ".f"
        else:
            th_rust, th_c = th, th + "f"
        nodes.append((feat, th_rust, th_c, int(l.group(1)), int(r.group(1)), fr.group(1) == "true"))
    return nodes


def write_rust(nodes: list[tuple[int, str, str, int, int, bool]], out: pathlib.Path) -> None:
    rs_lines = [
        "// @generated - do not edit; run: python scripts/gen_decision_tree.py",
        "pub const LEAF: u8 = 255;",
        "pub const FEATURE_COUNT: usize = 21;",
        "",
        "#[repr(C)]",
        "#[derive(Copy, Clone)]",
        "pub struct Node {",
        "    pub feature: u8,",
        "    pub threshold: f32,",
        "    pub left: i16,",
        "    pub right: i16,",
        "    pub fraud: u8,",
        "}",
        "",
        "#[inline]",
        "pub fn predict(features: &[f32; FEATURE_COUNT]) -> bool {",
        "    let mut index: usize = 0;",
        "    loop {",
        "        let node = &NODES[index];",
        "        if node.feature == LEAF {",
        "            return node.fraud != 0;",
        "        }",
        "        let v = features[node.feature as usize];",
        "        let next = if v <= node.threshold { node.left } else { node.right };",
        "        index = next as usize;",
        "    }",
        "}",
        "",
        "pub static NODES: &[Node] = &[",
    ]
    for feat, th, _, le, ri, fr in nodes:
        rs_lines.append(
            f"    Node {{ feature: {feat}, threshold: {th}, left: {le}, right: {ri}, fraud: {1 if fr else 0} }},"
        )
    rs_lines.append("];")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(rs_lines) + "\n", encoding="utf-8")


def write_c(nodes: list[tuple[int, str, str, int, int, bool]], c_root: pathlib.Path) -> None:
    h = """#ifndef DECISION_TREE_H
#define DECISION_TREE_H
#include <stdint.h>
#define TREE_LEAF 255
#define TREE_FEATURE_COUNT 21
typedef struct {
    uint8_t feature;
    float threshold;
    int16_t left, right;
    uint8_t fraud;
} tree_node_t;
extern const tree_node_t tree_nodes[];
extern const unsigned tree_node_count;
int tree_predict(const float features[TREE_FEATURE_COUNT]);
#endif
"""
    c = ['#include "decision_tree.h"', "", "const tree_node_t tree_nodes[] = {"]
    for feat, _, th, le, ri, fr in nodes:
        c.append(f"    {{ {feat}, {th}, {le}, {ri}, {1 if fr else 0} }},")
    c.append("};")
    c.append(f"const unsigned tree_node_count = {len(nodes)};")
    c.append("")
    c.append("int tree_predict(const float features[TREE_FEATURE_COUNT])")
    c.append("{")
    c.append("    unsigned index = 0;")
    c.append("    for (;;) {")
    c.append("        const tree_node_t *n = &tree_nodes[index];")
    c.append("        if (n->feature == TREE_LEAF)")
    c.append("            return n->fraud;")
    c.append(
        "        index = (features[n->feature] <= n->threshold) ? (unsigned)n->left : (unsigned)n->right;"
    )
    c.append("    }")
    c.append("}")

    h_path = c_root / "include" / "decision_tree.h"
    c_path = c_root / "src" / "decision_tree.c"
    h_path.parent.mkdir(parents=True, exist_ok=True)
    c_path.parent.mkdir(parents=True, exist_ok=True)
    h_path.write_text(h + "\n", encoding="utf-8")
    c_path.write_text("\n".join(c) + "\n", encoding="utf-8")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--c-dir",
        type=pathlib.Path,
        default=None,
        help="Raiz do repo C (escreve include/decision_tree.h e src/decision_tree.c). "
        "Padrão: env C_TREE_DIR ou ./c-tree",
    )
    ap.add_argument(
        "--rust-only",
        action="store_true",
        help="Gera só src/search/decision_tree.rs",
    )
    ap.add_argument(
        "--c-only",
        action="store_true",
        help="Gera só os arquivos C (não altera o .rs)",
    )
    args = ap.parse_args()

    src = (ROOT / "scripts" / "decision_tree.nodes").read_text(encoding="utf-8")
    nodes = parse_nodes(src)

    if not args.c_only:
        rust_out = ROOT / "src/search/decision_tree.rs"
        write_rust(nodes, rust_out)
        print(f"ok {len(nodes)} nodes -> {rust_out.relative_to(ROOT)}")

    if not args.rust_only:
        c_root = args.c_dir
        if c_root is None:
            env = os.environ.get("C_TREE_DIR")
            c_root = pathlib.Path(env) if env else ROOT / "c-tree"
        c_root = c_root.resolve()
        write_c(nodes, c_root)
        print(
            f"ok {len(nodes)} nodes -> {c_root / 'include/decision_tree.h'}, "
            f"{c_root / 'src/decision_tree.c'}"
        )


if __name__ == "__main__":
    main()
