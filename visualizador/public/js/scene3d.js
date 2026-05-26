import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { CSS2DRenderer, CSS2DObject } from "three/addons/renderers/CSS2DRenderer.js";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { UnrealBloomPass } from "three/addons/postprocessing/UnrealBloomPass.js";
import {
  buildLaptop,
  buildNetworkSwitch,
  buildCableSocket,
  buildServer,
  buildHttpPacket,
  buildJsonBraces,
  buildCpuChip,
  buildShield,
  buildAlert,
  buildDecisionGraph,
  buildGauge,
  buildResponsePacket,
  buildDataPacket,
  buildPcbFloor,
  ledMat,
} from "./tech-models.js";

export const PATH_COLORS = {
  ObviousLegit: 0x4ade80,
  ObviousFraud: 0xfb7185,
  Tree: 0xc084fc,
  Ratio: 0xfb923c,
};

/** Infra à esquerda (z=0); tier_score em faixa lateral separada (z negativo) */
export const LAYOUT_CENTER = new THREE.Vector3(4, 1.5, -1.5);
export const LAYOUT_HALF = { x: 20, z: 8 };
export const SCORER_Z = -5.5;
export const SCORER_STEP = 3.8;
export const BRANCH_Z = 2.6;

export const ARCH = {
  client: new THREE.Vector3(-14, 1.5, 0),
  lb: new THREE.Vector3(-10.5, 1.5, 0),
  socket: new THREE.Vector3(-7, 1.5, 0),
  api1: new THREE.Vector3(-3.5, 1.5, -3.5),
  api2: new THREE.Vector3(-3.5, 1.5, 3.5),
  http: new THREE.Vector3(-0.5, 1.5, 0),
  extract: new THREE.Vector3(2, 1.5, SCORER_Z),
  tier: new THREE.Vector3(2 + SCORER_STEP, 1.5, SCORER_Z),
  legit: new THREE.Vector3(2 + SCORER_STEP * 2, 1.5, SCORER_Z + BRANCH_Z),
  fraud: new THREE.Vector3(2 + SCORER_STEP * 2, 1.5, SCORER_Z - BRANCH_Z),
  tree: new THREE.Vector3(2 + SCORER_STEP * 3, 1.5, SCORER_Z),
  ratio: new THREE.Vector3(2 + SCORER_STEP * 4, 1.5, SCORER_Z),
  response: new THREE.Vector3(2 + SCORER_STEP * 5, 1.5, SCORER_Z),
};

function makeLabel(text, accent = "#38bdf8") {
  const el = document.createElement("div");
  el.className = "node-label-3d";
  el.innerHTML = `<span class="node-label-dot" style="background:${accent}"></span>${text}`;
  const obj = new CSS2DObject(el);
  obj.position.y = 1.35;
  return obj;
}

function addPlatform(group, color) {
  const plat = new THREE.Mesh(
    new THREE.CylinderGeometry(0.85, 0.95, 0.06, 32),
    new THREE.MeshStandardMaterial({
      color: 0x0f172a,
      emissive: new THREE.Color(color),
      emissiveIntensity: 0.38,
      metalness: 0.85,
      roughness: 0.4,
    }),
  );
  plat.position.y = -0.55;
  group.add(plat);
}

function createTechNode(id, pos, builder, labelText, accentHex) {
  const group = new THREE.Group();
  group.position.copy(pos);
  group.userData.id = id;
  group.userData.baseY = pos.y;

  const { group: model, primary } = builder();
  group.add(model);
  addPlatform(group, accentHex);

  const mat = primary.material;
  group.userData.mat = mat;
  group.userData.baseEmissive = mat.emissive.clone();
  group.userData.baseIntensity = mat.emissiveIntensity;
  group.userData.primary = primary;
  group.userData.model = model;

  group.add(makeLabel(labelText, accentHex));
  return group;
}

export function createScene(wrap) {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x0c1428);
  scene.fog = new THREE.FogExp2(0x0a1020, 0.012);

  const camera = new THREE.PerspectiveCamera(40, innerWidth / innerHeight, 0.1, 250);
  camera.position.set(4, 16, 26);

  const renderer = new THREE.WebGLRenderer({ antialias: true, powerPreference: "high-performance" });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.setSize(innerWidth, innerHeight);
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.22;
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  wrap.appendChild(renderer.domElement);

  const labelRenderer = new CSS2DRenderer();
  labelRenderer.setSize(innerWidth, innerHeight);
  labelRenderer.domElement.style.cssText = "position:absolute;inset:0;pointer-events:none";
  wrap.appendChild(labelRenderer.domElement);

  const composer = new EffectComposer(renderer);
  composer.addPass(new RenderPass(scene, camera));
  const bloom = new UnrealBloomPass(new THREE.Vector2(innerWidth, innerHeight), 0.42, 0.4, 0.4);
  composer.addPass(bloom);

  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.dampingFactor = 0.06;
  controls.target.copy(LAYOUT_CENTER);
  controls.maxPolarAngle = Math.PI * 0.52;
  controls.minDistance = 16;
  controls.maxDistance = 48;

  scene.add(new THREE.AmbientLight(0xb8d4f8, 0.42));
  scene.add(new THREE.HemisphereLight(0x93c5fd, 0x1e293b, 0.72));

  const sun = new THREE.DirectionalLight(0xffffff, 1.15);
  sun.position.set(6, 18, 10);
  scene.add(sun);

  const fill = new THREE.DirectionalLight(0x818cf8, 0.48);
  fill.position.set(-12, 8, -8);
  scene.add(fill);

  const rim = new THREE.DirectionalLight(0x5eead4, 0.28);
  rim.position.set(0, 6, -16);
  scene.add(rim);

  const overhead = new THREE.SpotLight(0xffffff, 1.75, 50, Math.PI / 3.5, 0.5, 0.6);
  overhead.position.set(2, 20, 6);
  overhead.target.position.copy(LAYOUT_CENTER);
  scene.add(overhead, overhead.target);

  const lights = {
    client: addPoint(0x38bdf8, ARCH.client),
    lb: addPoint(0xfbbf24, ARCH.lb),
    api: addPoint(0x818cf8, new THREE.Vector3(-3.5, 1.5, 0)),
    scorer: addPoint(0xc084fc, ARCH.tier),
  };
  function addPoint(color, pos) {
    const l = new THREE.PointLight(color, 2.1, 20, 1.5);
    l.position.copy(pos).add(new THREE.Vector3(0, 0.8, 0));
    scene.add(l);
    return l;
  }

  scene.add(buildPcbFloor(LAYOUT_CENTER.x, LAYOUT_CENTER.z));

  const hx = LAYOUT_HALF.x;
  const hz = LAYOUT_HALF.z;
  const cx = LAYOUT_CENTER.x;
  const cz = LAYOUT_CENTER.z;
  const y = -1.33;
  const boundsPts = [
    new THREE.Vector3(cx - hx, y, cz - hz),
    new THREE.Vector3(cx + hx, y, cz - hz),
    new THREE.Vector3(cx + hx, y, cz + hz),
    new THREE.Vector3(cx - hx, y, cz + hz),
    new THREE.Vector3(cx - hx, y, cz - hz),
  ];
  const boundsGeo = new THREE.BufferGeometry().setFromPoints(boundsPts);
  scene.add(
    new THREE.Line(
      boundsGeo,
      new THREE.LineBasicMaterial({
        color: 0x3b82f6,
        transparent: true,
        opacity: 0.35,
      }),
    ),
  );

  const nodeGroups = [];
  const register = (g) => {
    scene.add(g);
    nodeGroups.push(g);
    return g;
  };

  register(createTechNode("client", ARCH.client, () => buildLaptop(0x38bdf8), "Cliente / k6", "#38bdf8"));
  register(createTechNode("lb", ARCH.lb, () => buildNetworkSwitch(0xfbbf24), "Load Balancer :9999", "#fbbf24"));
  register(createTechNode("socket", ARCH.socket, () => buildCableSocket(0x94a3b8), "Unix socket FD", "#94a3b8"));
  register(createTechNode("api1", ARCH.api1, () => buildServer(0x6366f1), "API 1 — server", "#818cf8"));
  register(createTechNode("api2", ARCH.api2, () => buildServer(0x8b5cf6), "API 2 — server", "#a78bfa"));
  register(createTechNode("http", ARCH.http, () => buildHttpPacket(0x3b82f6), "HTTP parse", "#60a5fa"));
  register(createTechNode("extract", ARCH.extract, () => buildJsonBraces(0x22d3ee), "JSON extract", "#22d3ee"));
  register(createTechNode("tier", ARCH.tier, () => buildCpuChip(0xa855f7), "tier_score", "#c084fc"));
  register(createTechNode("legit", ARCH.legit, () => buildShield(0x22c55e), "Gasto seguro?", "#4ade80"));
  register(createTechNode("fraud", ARCH.fraud, () => buildAlert(0xef4444), "Gasto arriscado?", "#f87171"));
  register(createTechNode("tree", ARCH.tree, () => buildDecisionGraph(0xa855f7), "Árvore de decisão", "#d8b4fe"));
  register(createTechNode("ratio", ARCH.ratio, () => buildGauge(0xf97316), "Ratio fallback", "#fb923c"));
  register(createTechNode("response", ARCH.response, () => buildResponsePacket(0x2dd4bf), "Resposta HTTP", "#5eead4"));

  // Luz pontual em cada nó (ilumina de baixo)
  for (const group of nodeGroups) {
    const pos = new THREE.Vector3();
    group.getWorldPosition(pos);
    const accent = group.userData.mat?.emissive?.getHex?.() ?? 0x60a5fa;
    const uplight = new THREE.PointLight(accent, 0.85, 5, 2);
    uplight.position.set(pos.x, pos.y - 0.3, pos.z);
    group.add(uplight);
  }

  // Cabos de rede (fibra)
  const cableCore = new THREE.MeshBasicMaterial({ color: 0x7dd3fc, transparent: true, opacity: 0.55 });
  const cableGlow = new THREE.MeshBasicMaterial({
    color: 0xbae6fd,
    transparent: true,
    opacity: 0.22,
    blending: THREE.AdditiveBlending,
  });

  /** Faixa de retorno — lado oposto ao tier_score (SCORER_Z < 0) */
  const RETURN_LANE_Z = 6;
  const RETURN_LANE_Y = 1.12;

  function addCableCurve(curve, matCore = cableCore, matGlow = cableGlow) {
    const core = new THREE.Mesh(new THREE.TubeGeometry(curve, 48, 0.028, 6, false), matCore);
    const glow = new THREE.Mesh(new THREE.TubeGeometry(curve, 48, 0.055, 6, false), matGlow);
    scene.add(core, glow);
  }

  function networkCable(from, to, arc = 0.3) {
    const mid = from.clone().add(to).multiplyScalar(0.5);
    const dist = from.distanceTo(to);
    mid.y += dist * arc * 0.15;
    if (Math.abs(from.z - to.z) > 0.5) {
      mid.z = (from.z + to.z) * 0.5;
    }
    const curve = new THREE.QuadraticBezierCurve3(from.clone(), mid, to.clone());
    addCableCurve(curve);
    for (const p of [from, to]) {
      const plug = new THREE.Mesh(
        new THREE.BoxGeometry(0.12, 0.08, 0.12),
        ledMat(0x60a5fa, 1),
      );
      plug.position.copy(p);
      scene.add(plug);
    }
  }

  function pathFromPoints(pts) {
    const path = new THREE.CurvePath();
    for (let i = 0; i < pts.length - 1; i++) {
      path.add(new THREE.LineCurve3(pts[i], pts[i + 1]));
    }
    return path;
  }

  /** Retorno em L: Z → lateral → cliente (segmentos retos, sem arco) */
  function returnPathPoints(from, to) {
    const y = RETURN_LANE_Y;
    const z = RETURN_LANE_Z;
    return [
      from.clone(),
      new THREE.Vector3(from.x, from.y, z),
      new THREE.Vector3(from.x, y, z),
      new THREE.Vector3(to.x, y, z),
      new THREE.Vector3(to.x, to.y, z),
      to.clone(),
    ];
  }

  function returnCable(from, to) {
    const pts = returnPathPoints(from, to);
    const path = pathFromPoints(pts);
    const retCore = new THREE.MeshBasicMaterial({
      color: 0x5eead4,
      transparent: true,
      opacity: 0.45,
    });
    const retGlow = new THREE.MeshBasicMaterial({
      color: 0x99f6e4,
      transparent: true,
      opacity: 0.18,
      blending: THREE.AdditiveBlending,
    });
    scene.add(
      new THREE.Mesh(new THREE.TubeGeometry(path, 64, 0.026, 5, false), retCore),
      new THREE.Mesh(new THREE.TubeGeometry(path, 64, 0.048, 5, false), retGlow),
    );
    const laneGeo = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(pts[2].x, -1.32, RETURN_LANE_Z),
      new THREE.Vector3(pts[3].x, -1.32, RETURN_LANE_Z),
    ]);
    scene.add(
      new THREE.Line(
        laneGeo,
        new THREE.LineBasicMaterial({ color: 0x2dd4bf, transparent: true, opacity: 0.2 }),
      ),
    );
    for (const p of [pts[0], pts[pts.length - 1]]) {
      const plug = new THREE.Mesh(
        new THREE.BoxGeometry(0.1, 0.07, 0.1),
        ledMat(0x2dd4bf, 0.9),
      );
      plug.position.copy(p);
      scene.add(plug);
    }
  }

  function returnWaypoints(from, to) {
    return returnPathPoints(from, to);
  }

  /** Após tier_score, só um ramo conforme o path (nunca legit + fraud na mesma animação) */
  function routeAfterTier(path) {
    switch (path) {
      case "ObviousLegit":
        return [ARCH.legit.clone(), ARCH.response.clone()];
      case "ObviousFraud":
        return [ARCH.fraud.clone(), ARCH.response.clone()];
      case "Tree":
        return [ARCH.tree.clone(), ARCH.response.clone()];
      default:
        return [ARCH.ratio.clone(), ARCH.response.clone()];
    }
  }

  function buildAnimationPoints(flowSteps, path) {
    const tierIdx = flowSteps.findIndex((s) => s.id === "tier");
    const points = [];
    const ingress = tierIdx >= 0 ? flowSteps.slice(0, tierIdx + 1) : flowSteps;
    for (const step of ingress) {
      if (step.id === "client_out") continue;
      points.push(resolveStepPos(step));
    }
    if (tierIdx >= 0) points.push(...routeAfterTier(path));
    if (flowSteps.some((s) => s.id === "client_out")) {
      points.push(...returnWaypoints(ARCH.response, ARCH.client));
    }
    return points;
  }

  const links = [
    [ARCH.client, ARCH.lb, 0.08],
    [ARCH.lb, ARCH.socket, 0.08],
    [ARCH.socket, ARCH.api1, 0.22],
    [ARCH.socket, ARCH.api2, 0.22],
    [ARCH.api1, ARCH.http, 0.18],
    [ARCH.api2, ARCH.http, 0.18],
    [ARCH.http, ARCH.extract, 0.2],
    [ARCH.extract, ARCH.tier, 0.06],
    [ARCH.tier, ARCH.tree, 0.08],
    [ARCH.tree, ARCH.ratio, 0.06],
    [ARCH.ratio, ARCH.response, 0.04],
  ];

  const shortcutCore = new THREE.MeshBasicMaterial({
    color: 0x4ade80,
    transparent: true,
    opacity: 0.32,
  });
  const shortcutGlow = new THREE.MeshBasicMaterial({
    color: 0x86efac,
    transparent: true,
    opacity: 0.14,
    blending: THREE.AdditiveBlending,
  });
  function makeShortcutCable(from, to) {
    const y = from.y - 0.45;
    const z = from.z;
    const pts = [
      from.clone(),
      new THREE.Vector3(from.x, y, z),
      new THREE.Vector3(to.x, y, z),
      to.clone(),
    ];
    const path = pathFromPoints(pts);
    const group = new THREE.Group();
    group.add(
      new THREE.Mesh(new THREE.TubeGeometry(path, 32, 0.022, 5, false), shortcutCore),
      new THREE.Mesh(new THREE.TubeGeometry(path, 32, 0.04, 5, false), shortcutGlow),
    );
    group.visible = false;
    scene.add(group);
    return group;
  }

  const shortcutLegit = makeShortcutCable(ARCH.legit, ARCH.response);
  const shortcutFraud = makeShortcutCable(ARCH.fraud, ARCH.response);

  const branchCableMat = new THREE.MeshBasicMaterial({
    color: 0xc084fc,
    transparent: true,
    opacity: 0.55,
  });
  let activeBranchCable = null;

  function setBranchCables(path) {
    if (activeBranchCable) {
      scene.remove(activeBranchCable);
      activeBranchCable = null;
    }
    shortcutLegit.visible = path === "ObviousLegit";
    shortcutFraud.visible = path === "ObviousFraud";

    let from;
    if (path === "ObviousLegit") from = ARCH.legit;
    else if (path === "ObviousFraud") from = ARCH.fraud;
    else return;

    const pathCurve = pathFromPoints([ARCH.tier.clone(), from.clone()]);
    activeBranchCable = new THREE.Mesh(
      new THREE.TubeGeometry(pathCurve, 24, 0.03, 5, false),
      branchCableMat,
    );
    scene.add(activeBranchCable);
  }

  for (const [a, b, arc] of links) networkCable(a, b, arc ?? 0.08);
  returnCable(ARCH.response, ARCH.client);

  function fitCamera() {
    const box = new THREE.Box3();
    for (const p of Object.values(ARCH)) box.expandByPoint(p);
    box.expandByScalar(2.5);
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3());
    const maxDim = Math.max(size.x, size.z, 8);
    const dist = maxDim / (2 * Math.tan((camera.fov * Math.PI) / 360)) + 4;
    camera.position.set(center.x, center.y + dist * 0.55, center.z + dist);
    controls.target.copy(center);
    controls.update();
  }
  fitCamera();

  const particleGroup = buildDataPacket(0xffffff);
  particleGroup.visible = false;
  scene.add(particleGroup);
  const particleLight = new THREE.PointLight(0xffffff, 2.5, 8, 2);
  particleGroup.add(particleLight);

  let flowAnim = null;
  const clock = new THREE.Clock();

  function resolveStepPos(step) {
    const id = step.id;
    if (id === "client" || id === "client_out") return ARCH.client.clone();
    if (id === "lb") return ARCH.lb.clone();
    if (id === "socket") return ARCH.socket.clone();
    if (id === "api1" || id === "api2") return (ARCH[id] ?? ARCH.api1).clone();
    if (id === "http") return ARCH.http.clone();
    if (id === "extract") return ARCH.extract.clone();
    if (id === "tier") return ARCH.tier.clone();
    if (id.startsWith("legit")) return ARCH.legit.clone();
    if (id.startsWith("fraud")) return ARCH.fraud.clone();
    if (id === "tree" || id.startsWith("tree")) return ARCH.tree.clone();
    if (id === "ratio") return ARCH.ratio.clone();
    if (id === "response") return ARCH.response.clone();
    return ARCH.extract.clone();
  }

  const scorerBranches = new Set(["legit", "fraud", "tree", "ratio"]);

  function highlightPath(flowSteps, pathColor, tierPath) {
    const activeIds = new Set(flowSteps.map((s) => s.id));
    if (tierPath === "ObviousLegit") activeIds.delete("fraud");
    if (tierPath === "ObviousFraud") activeIds.delete("legit");
    if (tierPath === "Tree" || tierPath === "Ratio") {
      activeIds.delete("legit");
      activeIds.delete("fraud");
    }
    setBranchCables(tierPath);
    const c = new THREE.Color(pathColor);
    for (const group of nodeGroups) {
      const id = group.userData.id;
      const active = activeIds.has(id);
      const mat = group.userData.mat;
      if (!mat) continue;
      const isIdleBranch = scorerBranches.has(id) && !active;
      if (active) {
        mat.emissive.copy(c);
        mat.emissiveIntensity = 1.2;
        mat.opacity = 1;
        mat.transparent = false;
        group.scale.setScalar(1.1);
        group.userData.model.traverse((o) => {
          if (o.userData.isLed && o.material) {
            o.material.emissive.copy(c);
            o.material.emissiveIntensity = 2.5;
          }
        });
      } else if (isIdleBranch) {
        mat.emissiveIntensity = 0.06;
        mat.opacity = 0.22;
        mat.transparent = true;
        group.scale.setScalar(0.88);
      } else {
        mat.emissiveIntensity = 0.18;
        mat.opacity = 0.45;
        mat.transparent = true;
        group.scale.setScalar(0.92);
      }
    }
    particleLight.color.copy(c);
    bloom.strength = 0.58;
  }

  function resetHighlights() {
    shortcutLegit.visible = false;
    shortcutFraud.visible = false;
    if (activeBranchCable) {
      scene.remove(activeBranchCable);
      activeBranchCable = null;
    }
    for (const group of nodeGroups) {
      const mat = group.userData.mat;
      if (!mat) continue;
      mat.emissive.copy(group.userData.baseEmissive);
      mat.emissiveIntensity = group.userData.baseIntensity;
      mat.opacity = 1;
      mat.transparent = false;
      group.scale.setScalar(1);
      group.userData.model.traverse((o) => {
        if (o.userData.isLed && o.material?.emissive) {
          o.material.emissiveIntensity = 1.5;
        }
      });
    }
    bloom.strength = 0.42;
  }

  function animateFlow(event) {
    if (flowAnim?.raf) cancelAnimationFrame(flowAnim.raf);
    const { trace } = event;
    if (!trace?.ok) return;

    const color = PATH_COLORS[trace.path] ?? 0xffffff;
    particleGroup.traverse((o) => {
      if (o.isMesh && o.material?.emissive) o.material.emissive.setHex(color);
    });
    particleGroup.visible = true;
    highlightPath(trace.flowSteps, color, trace.path);

    const points = buildAnimationPoints(trace.flowSteps, trace.path);

    let step = 0;
    let t = 0;

    function tick() {
      t += 0.018;
      if (t >= 1) {
        t = 0;
        step++;
        if (step >= points.length - 1) {
          particleGroup.visible = false;
          setTimeout(resetHighlights, 1400);
          return;
        }
      }
      const eased = easeInOutCubic(t);
      particleGroup.position.lerpVectors(points[step], points[step + 1], eased);
      particleGroup.rotation.y += 0.12;
      particleLight.intensity = 1.5 + eased * 1.2;
      flowAnim = { raf: requestAnimationFrame(tick) };
    }
    tick();
  }

  function easeInOutCubic(t) {
    return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
  }

  function onResize() {
    const w = innerWidth;
    const h = innerHeight;
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
    renderer.setSize(w, h);
    composer.setSize(w, h);
    bloom.resolution.set(w, h);
    labelRenderer.setSize(w, h);
  }

  function render() {
    const t = clock.getElapsedTime();
    for (const group of nodeGroups) {
      group.position.y = group.userData.baseY + Math.sin(t * 0.8 + group.position.x * 0.3) * 0.02;
      const model = group.userData.model;
      if (model?.userData?.fan) model.userData.fan.rotation.z = t * 3;
      if (model?.userData?.needle) {
        model.userData.needle.rotation.z = 0.25 + Math.sin(t * 1.5) * 0.35;
      }
    }
    lights.client.intensity = 2 + Math.sin(t * 2.5) * 0.25;
    lights.lb.intensity = 1.85 + Math.sin(t * 2 + 1) * 0.2;
    lights.api.intensity = 2;
    lights.scorer.intensity = 1.9 + Math.sin(t * 1.7 + 2) * 0.15;
    overhead.intensity = 1.65 + Math.sin(t * 0.8) * 0.12;
    controls.update();
    composer.render();
    labelRenderer.render(scene, camera);
  }

  return { render, onResize, animateFlow, fitCamera };
}
