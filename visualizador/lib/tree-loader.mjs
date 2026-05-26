import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const NODES_PATH = join(ROOT, "scripts", "decision_tree.nodes");

/** @typedef {{ feature: number, threshold: number, left: number, right: number, fraud: boolean }} TreeNode */

/** @returns {TreeNode[]} */
export function loadDecisionTree() {
  const src = readFileSync(NODES_PATH, "utf8");
  const nodes = [];
  const re =
    /\.\{\s*\.feature\s*=\s*(\d+|LeafFeature),\s*\.threshold\s*=\s*([^,]+),\s*\.left\s*=\s*(-?\d+),\s*\.right\s*=\s*(-?\d+),\s*\.fraud\s*=\s*(true|false)\s*\}/g;
  let m;
  while ((m = re.exec(src)) !== null) {
    nodes.push({
      feature: m[1] === "LeafFeature" ? 255 : Number(m[1]),
      threshold: Number(m[2]),
      left: Number(m[3]),
      right: Number(m[4]),
      fraud: m[5] === "true",
    });
  }
  if (nodes.length === 0) {
    throw new Error(`Nenhum nó em ${NODES_PATH}`);
  }
  return nodes;
}
