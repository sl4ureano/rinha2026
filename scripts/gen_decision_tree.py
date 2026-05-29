#!/usr/bin/env python3
"""Gera src/search/decision_tree.rs a partir de scripts/decision_tree.nodes."""
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
        th_rust = "0.0"
    elif "." not in th:
        th_rust = th + ".0"
    else:
        th_rust = th
    nodes.append((feat, th_rust, int(l.group(1)), int(r.group(1)), fr.group(1) == "true"))

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
for feat, th, le, ri, fr in nodes:
    rs_lines.append(
        f"    Node {{ feature: {feat}, threshold: {th}, left: {le}, right: {ri}, fraud: {1 if fr else 0} }},"
    )
rs_lines.append("];")
(ROOT / "src/search/decision_tree.rs").write_text("\n".join(rs_lines) + "\n")
print(f"ok {len(nodes)} nodes (Rust); C: https://github.com/adsanla/rinha2026")
