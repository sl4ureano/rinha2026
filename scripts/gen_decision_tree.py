#!/usr/bin/env python3
"""Gera decision_tree (.rs e .c) a partir de scripts/decision_tree.nodes."""
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
src = (ROOT / "scripts" / "decision_tree.nodes").read_text()
m = re.search(r"const nodes = \[_\]Node\{(.*?)\};", src, re.S)
body = m.group(1)
nodes = []
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
(ROOT / "src/search/decision_tree.rs").write_text("\n".join(rs_lines) + "\n")

# C
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
(ROOT / "VERSAO-c/include/decision_tree.h").write_text(h + "\n")
(ROOT / "VERSAO-c/src/decision_tree.c").write_text("\n".join(c) + "\n")
print(f"ok {len(nodes)} nodes")
