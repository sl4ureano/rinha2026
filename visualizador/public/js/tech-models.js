import * as THREE from "three";
import { RoundedBoxGeometry } from "three/addons/geometries/RoundedBoxGeometry.js";

/** Materiais estilo datacenter / PCB */
export function techPanel(color, opts = {}) {
  const c = new THREE.Color(color);
  return new THREE.MeshStandardMaterial({
    color: c.multiplyScalar(0.48),
    emissive: c,
    emissiveIntensity: opts.emissive ?? 0.52,
    metalness: 0.82,
    roughness: 0.28,
  });
}

export function techDark(color = 0x334155) {
  return new THREE.MeshStandardMaterial({
    color,
    emissive: new THREE.Color(color),
    emissiveIntensity: 0.08,
    metalness: 0.85,
    roughness: 0.32,
  });
}

export function ledMat(color, intensity = 2.4) {
  return new THREE.MeshStandardMaterial({
    color,
    emissive: color,
    emissiveIntensity: intensity,
    metalness: 0.2,
    roughness: 0.4,
  });
}

export function screenMat(color) {
  return new THREE.MeshStandardMaterial({
    color: 0x020617,
    emissive: color,
    emissiveIntensity: 1.05,
    metalness: 0.1,
    roughness: 0.2,
  });
}

function ledRow(parent, count, y, z, color, spread = 0.12) {
  for (let i = 0; i < count; i++) {
    const led = new THREE.Mesh(
      new THREE.BoxGeometry(0.06, 0.06, 0.02),
      ledMat(color, 1.5),
    );
    led.position.set((i - (count - 1) / 2) * spread, y, z);
    led.userData.isLed = true;
    parent.add(led);
  }
}

/** Cliente — notebook */
export function buildLaptop(accent = 0x38bdf8) {
  const g = new THREE.Group();
  const body = new THREE.Mesh(
    new RoundedBoxGeometry(1.4, 0.08, 1, 3, 0.03),
    techDark(0x1e293b),
  );
  body.position.y = -0.2;
  const screen = new THREE.Mesh(
    new RoundedBoxGeometry(1.35, 0.85, 0.06, 3, 0.03),
    techDark(0x0f172a),
  );
  screen.position.set(0, 0.35, -0.08);
  screen.rotation.x = -0.18;
  const display = new THREE.Mesh(
    new RoundedBoxGeometry(1.2, 0.68, 0.02, 2, 0.01),
    screenMat(accent),
  );
  display.position.set(0, 0.38, -0.1);
  display.rotation.x = -0.18;
  const kb = new THREE.Mesh(
    new THREE.PlaneGeometry(1.1, 0.55),
    techPanel(accent, { emissive: 0.15 }),
  );
  kb.rotation.x = -Math.PI / 2;
  kb.position.set(0, -0.16, 0.02);
  g.add(body, screen, display, kb);
  return { group: g, primary: display };
}

/** Load balancer — switch de rede */
export function buildNetworkSwitch(accent = 0xfbbf24) {
  const g = new THREE.Group();
  const chassis = new THREE.Mesh(
    new RoundedBoxGeometry(1.5, 0.35, 1, 4, 0.06),
    techPanel(accent),
  );
  for (let i = 0; i < 8; i++) {
    const port = new THREE.Mesh(
      new THREE.BoxGeometry(0.1, 0.12, 0.14),
      techDark(0x334155),
    );
    port.position.set((i - 3.5) * 0.16, -0.08, 0.42);
    g.add(port);
  }
  ledRow(g, 5, 0.12, 0.48, accent);
  const antenna = new THREE.Mesh(
    new THREE.CylinderGeometry(0.02, 0.02, 0.5, 8),
    techDark(),
  );
  antenna.position.set(0.55, 0.35, -0.35);
  const antenna2 = antenna.clone();
  antenna2.position.set(-0.55, 0.35, -0.35);
  g.add(chassis, antenna, antenna2);
  return { group: g, primary: chassis };
}

/** Unix socket — plugue + cabo + porta */
export function buildCableSocket(accent = 0x94a3b8) {
  const g = new THREE.Group();
  const port = new THREE.Mesh(
    new RoundedBoxGeometry(0.5, 0.35, 0.25, 3, 0.04),
    techDark(0x334155),
  );
  const plug = new THREE.Mesh(
    new RoundedBoxGeometry(0.28, 0.2, 0.35, 2, 0.03),
    techPanel(accent),
  );
  plug.position.set(0.45, 0, 0.2);
  const cable = new THREE.Mesh(
    new THREE.CylinderGeometry(0.05, 0.05, 0.9, 10),
    techDark(0x475569),
  );
  cable.rotation.z = Math.PI / 2;
  cable.position.set(0.95, 0, 0.2);
  const pins = new THREE.Mesh(
    new THREE.BoxGeometry(0.22, 0.08, 0.06),
    ledMat(accent, 1.2),
  );
  pins.position.set(0.45, 0, 0.38);
  g.add(port, plug, cable, pins);
  return { group: g, primary: plug };
}

/** Servidor — rack 1U com ventoinha e LEDs */
export function buildServer(accent = 0x6366f1) {
  const g = new THREE.Group();
  const chassis = new THREE.Mesh(
    new RoundedBoxGeometry(1.2, 0.55, 1.4, 4, 0.05),
    techPanel(accent),
  );
  const face = new THREE.Mesh(
    new RoundedBoxGeometry(1.05, 0.45, 0.04, 3, 0.02),
    techDark(0x0f172a),
  );
  face.position.z = 0.68;
  const fan = new THREE.Mesh(
    new THREE.CylinderGeometry(0.22, 0.22, 0.04, 16),
    techDark(0x1e293b),
  );
  fan.rotation.x = Math.PI / 2;
  fan.position.set(0, 0, 0.7);
  const hub = new THREE.Mesh(
    new THREE.CylinderGeometry(0.06, 0.06, 0.05, 8),
    ledMat(0x64748b, 0.8),
  );
  hub.rotation.x = Math.PI / 2;
  hub.position.set(0, 0, 0.72);
  ledRow(g, 4, 0.18, 0.72, 0x22c55e, 0.18);
  const slot = new THREE.Mesh(
    new THREE.BoxGeometry(0.9, 0.04, 0.02),
    ledMat(accent, 0.6),
  );
  slot.position.set(0, -0.12, 0.71);
  g.add(chassis, face, fan, hub, slot);
  g.userData.fan = fan;
  return { group: g, primary: chassis };
}

/** HTTP — pacote de request */
export function buildHttpPacket(accent = 0x64748b) {
  const g = new THREE.Group();
  const env = new THREE.Mesh(
    new RoundedBoxGeometry(0.9, 0.55, 0.12, 3, 0.03),
    techPanel(accent),
  );
  const header = new THREE.Mesh(
    new THREE.BoxGeometry(0.75, 0.1, 0.02),
    ledMat(0x3b82f6, 1.2),
  );
  header.position.set(0, 0.12, 0.07);
  const lines = [];
  for (let i = 0; i < 3; i++) {
    const line = new THREE.Mesh(
      new THREE.BoxGeometry(0.55 - i * 0.08, 0.04, 0.02),
      techDark(0x475569),
    );
    line.position.set(-0.05, -0.02 - i * 0.1, 0.07);
    lines.push(line);
  }
  const arrow = new THREE.Mesh(
    new THREE.ConeGeometry(0.12, 0.28, 4),
    ledMat(0x60a5fa, 1.5),
  );
  arrow.rotation.z = -Math.PI / 2;
  arrow.position.set(0.55, 0, 0);
  g.add(env, header, ...lines, arrow);
  return { group: g, primary: env };
}

/** JSON — chaves { } */
export function buildJsonBraces(accent = 0x22d3ee) {
  const g = new THREE.Group();
  const mat = techPanel(accent);
  const barGeo = new THREE.BoxGeometry(0.08, 0.55, 0.12);
  const left = new THREE.Group();
  const l1 = new THREE.Mesh(barGeo, mat);
  l1.position.set(-0.2, 0.15, 0);
  const l2 = new THREE.Mesh(barGeo, mat);
  l2.position.set(-0.2, -0.15, 0);
  const l3 = new THREE.Mesh(new THREE.BoxGeometry(0.28, 0.08, 0.12), mat);
  l3.position.set(-0.08, 0.28, 0);
  const l4 = l3.clone();
  l4.position.set(-0.08, -0.28, 0);
  left.add(l1, l2, l3, l4);
  const right = left.clone();
  right.scale.x = -1;
  const core = new THREE.Mesh(
    new THREE.BoxGeometry(0.35, 0.5, 0.08),
    screenMat(accent),
  );
  g.add(left, right, core);
  return { group: g, primary: core };
}

/** CPU / chip — tier_score */
export function buildCpuChip(accent = 0xa855f7) {
  const g = new THREE.Group();
  const die = new THREE.Mesh(
    new RoundedBoxGeometry(0.85, 0.1, 0.85, 3, 0.02),
    techPanel(accent),
  );
  const surface = new THREE.Mesh(
    new THREE.BoxGeometry(0.65, 0.02, 0.65),
    screenMat(accent),
  );
  surface.position.y = 0.06;
  const pinMat = techDark(0xc4b5fd);
  for (let i = 0; i < 12; i++) {
    const pin = new THREE.Mesh(new THREE.BoxGeometry(0.04, 0.08, 0.02), pinMat);
    const side = i < 6 ? -1 : 1;
    const idx = i % 6;
    pin.position.set((idx - 2.5) * 0.14, -0.06, side * 0.48);
    g.add(pin);
  }
  for (let i = 0; i < 12; i++) {
    const pin = new THREE.Mesh(new THREE.BoxGeometry(0.02, 0.08, 0.04), pinMat);
    const side = i < 6 ? -1 : 1;
    const idx = i % 6;
    pin.position.set(side * 0.48, -0.06, (idx - 2.5) * 0.14);
    g.add(pin);
  }
  g.add(die, surface);
  return { group: g, primary: die };
}

/** Escudo — gasto seguro (cilindro + cúpula + check) */
export function buildShield(accent = 0x22c55e) {
  const g = new THREE.Group();
  const bodyMat = techPanel(accent, { emissive: 0.62 });
  bodyMat.side = THREE.DoubleSide;

  const shieldBody = new THREE.Mesh(
    new THREE.CylinderGeometry(0.48, 0.54, 0.78, 6, 1),
    bodyMat,
  );
  shieldBody.scale.z = 0.38;

  const topCap = new THREE.Mesh(
    new THREE.SphereGeometry(0.5, 16, 10, 0, Math.PI * 2, 0, Math.PI / 2),
    bodyMat,
  );
  topCap.position.y = 0.4;

  const border = new THREE.Mesh(
    new THREE.TorusGeometry(0.4, 0.045, 8, 6, Math.PI),
    techDark(0x14532d),
  );
  border.rotation.x = Math.PI / 2;
  border.position.set(0, 0.22, 0.3);

  const checkMat = ledMat(0x4ade80, 2.6);
  const armA = new THREE.Mesh(new THREE.BoxGeometry(0.16, 0.05, 0.07), checkMat);
  armA.position.set(-0.1, 0.06, 0.34);
  armA.rotation.z = -0.65;
  const armB = new THREE.Mesh(new THREE.BoxGeometry(0.28, 0.05, 0.07), checkMat);
  armB.position.set(0.12, -0.04, 0.34);
  armB.rotation.z = 0.5;

  const lock = new THREE.Mesh(
    new THREE.BoxGeometry(0.14, 0.18, 0.06),
    techDark(0x166534),
  );
  lock.position.set(0, -0.28, 0.28);

  g.add(shieldBody, topCap, border, armA, armB, lock);
  shieldBody.userData.isCore = true;
  return { group: g, primary: shieldBody };
}

/** Alerta — gasto arriscado */
export function buildAlert(accent = 0xef4444) {
  const g = new THREE.Group();
  const tri = new THREE.Mesh(
    new THREE.ConeGeometry(0.5, 0.85, 3),
    techPanel(accent, { emissive: 0.7 }),
  );
  tri.rotation.y = Math.PI;
  const bang = new THREE.Mesh(
    new THREE.BoxGeometry(0.08, 0.35, 0.08),
    ledMat(0xfef08a, 2.5),
  );
  bang.position.y = 0.05;
  const dot = new THREE.Mesh(
    new THREE.SphereGeometry(0.06, 8, 8),
    ledMat(0xfef08a, 2.5),
  );
  dot.position.y = -0.2;
  g.add(tri, bang, dot);
  return { group: g, primary: tri };
}

/** Árvore de decisão — grafo neural */
export function buildDecisionGraph(accent = 0xa855f7) {
  const g = new THREE.Group();
  const nodeMat = ledMat(accent, 1.8);
  const edgeMat = new THREE.LineBasicMaterial({
    color: accent,
    transparent: true,
    opacity: 0.6,
  });
  const layers = [
    [{ x: 0, y: 0.5 }],
    [{ x: -0.35, y: 0.15 }, { x: 0.35, y: 0.15 }],
    [{ x: -0.5, y: -0.25 }, { x: 0, y: -0.25 }, { x: 0.5, y: -0.25 }],
    [{ x: -0.25, y: -0.55 }, { x: 0.25, y: -0.55 }],
  ];
  const nodes = [];
  for (const layer of layers) {
    for (const p of layer) {
      const n = new THREE.Mesh(new THREE.SphereGeometry(0.09, 12, 12), nodeMat);
      n.position.set(p.x, p.y, 0);
      g.add(n);
      nodes.push(n);
    }
  }
  const edges = [
    [0, 1], [0, 2], [1, 3], [1, 4], [2, 4], [2, 5], [3, 6], [4, 6], [4, 7], [5, 7],
  ];
  const positions = [];
  const flat = layers.flat();
  for (const [a, b] of edges) {
    const pa = flat[a];
    const pb = flat[b];
    positions.push(pa.x, pa.y, 0, pb.x, pb.y, 0);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  g.add(new THREE.LineSegments(geo, edgeMat));
  const board = new THREE.Mesh(
    new RoundedBoxGeometry(1.1, 1.2, 0.08, 3, 0.02),
    techDark(0x1e1b4b),
  );
  board.position.z = -0.08;
  g.add(board);
  return { group: g, primary: nodes[0] };
}

/** Ratio — medidor / gauge */
export function buildGauge(accent = 0xf97316) {
  const g = new THREE.Group();
  const base = new THREE.Mesh(
    new RoundedBoxGeometry(0.95, 0.55, 0.1, 3, 0.03),
    techDark(0x292524),
  );
  const arc = new THREE.Mesh(
    new THREE.TorusGeometry(0.32, 0.04, 8, 24, Math.PI),
    techPanel(accent),
  );
  arc.rotation.x = Math.PI / 2;
  arc.position.set(0, 0.08, 0.06);
  const needle = new THREE.Mesh(
    new THREE.BoxGeometry(0.04, 0.38, 0.02),
    ledMat(0xfbbf24, 2),
  );
  needle.position.set(0, 0.05, 0.08);
  needle.rotation.z = 0.4;
  needle.geometry.translate(0, 0.19, 0);
  g.userData.needle = needle;
  const label = new THREE.Mesh(
    new THREE.BoxGeometry(0.25, 0.06, 0.02),
    ledMat(accent, 1),
  );
  label.position.set(0, -0.15, 0.07);
  g.add(base, arc, needle, label);
  return { group: g, primary: arc };
}

/** Resposta — pacote JSON de saída */
export function buildResponsePacket(accent = 0x2dd4bf) {
  const g = new THREE.Group();
  const pkt = new THREE.Mesh(
    new RoundedBoxGeometry(1, 0.7, 0.14, 4, 0.04),
    techPanel(accent, { emissive: 0.6 }),
  );
  const stripe = new THREE.Mesh(
    new THREE.BoxGeometry(0.9, 0.12, 0.02),
    ledMat(accent, 1.5),
  );
  stripe.position.set(0, 0.18, 0.08);
  const ok = new THREE.Mesh(
    new THREE.RingGeometry(0.1, 0.14, 16),
    ledMat(0x4ade80, 2),
  );
  ok.position.set(0.3, -0.1, 0.08);
  const out = new THREE.Mesh(
    new THREE.ConeGeometry(0.1, 0.22, 4),
    ledMat(accent, 1.2),
  );
  out.rotation.z = -Math.PI / 2;
  out.position.set(0.58, 0, 0);
  g.add(pkt, stripe, ok, out);
  return { group: g, primary: pkt };
}

/** Partícula de dados — cubo + bits */
export function buildDataPacket(color = 0xffffff) {
  const g = new THREE.Group();
  const core = new THREE.Mesh(
    new RoundedBoxGeometry(0.22, 0.16, 0.28, 2, 0.03),
    ledMat(color, 2.5),
  );
  const bit = new THREE.Mesh(
    new THREE.BoxGeometry(0.04, 0.04, 0.04),
    ledMat(0x5eead4, 1.5),
  );
  bit.position.set(0.14, 0.1, 0);
  g.add(core, bit);
  return g;
}

/** Chão simples — superfície escura sem grade/trilhas */
export function buildPcbFloor(centerX = 0, centerZ = 0) {
  const g = new THREE.Group();
  const sizeX = 44;
  const sizeZ = 22;
  const base = new THREE.Mesh(
    new THREE.PlaneGeometry(sizeX, sizeZ),
    new THREE.MeshStandardMaterial({
      color: 0x0f172a,
      emissive: 0x0c1428,
      emissiveIntensity: 0.08,
      metalness: 0.6,
      roughness: 0.55,
    }),
  );
  base.rotation.x = -Math.PI / 2;
  base.position.set(centerX, -1.35, centerZ);
  g.add(base);
  return g;
}
