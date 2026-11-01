/**
 * CAD chapter: SolidDraft replays the bracket's feature tree. The sketch draws itself on a
 * plane, the part extrudes up through a clipping plane, five drills cut the holes, the edges
 * light up in a sweep, then the part lifts off the plane and turns free. Accent ice.
 *
 * The bracket is a real SolidDraft export (site/parts/bracket-plain.txt): a 120 x 90 x 10 plate,
 * a 10 x 90 x 60 flange on its -X end, a 24 mm boss with a 12 mm bore, four 9 mm holes and a
 * 40 x 12 slot through the flange.
 */
import * as THREE from "three";
import { Line2 } from "three/examples/jsm/lines/Line2.js";
import { LineGeometry } from "three/examples/jsm/lines/LineGeometry.js";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import { LineSegmentsGeometry } from "three/examples/jsm/lines/LineSegmentsGeometry.js";

import { fitPart } from "../assets";
import type { Chapter, ChapterContext } from "../types";

const ICE = "#5bc8f5";
/** 1 mm of the part in scene units: the 120 mm long side is fitted to 1.8. */
const MM = 1.8 / 120;
const PLANE_Y = 0.55;
/** Local heights of the fitted (centred) geometry: the part spans z 0..60 mm, so its centre is at 30. */
const PLATE_BOTTOM = -30 * MM;
const PLATE_TOP = -20 * MM;
const BOSS_TOP = 0;
const FLANGE_TOP = 30 * MM;
const PART_Y = PLANE_Y - PLATE_BOTTOM;
/** Part mm (x, y on the plate) to local (x, z): the GLB is Y up, so part y becomes -z. The part is centred on (60, 45). */
const toLocal = (x: number, y: number) => [(x - 60) * MM, (45 - y) * MM] as const;
const BOLTS = [toLocal(25, 23), toLocal(25, 67), toLocal(115, 23), toLocal(115, 67)];
const BOSS = toLocal(70, 45);
const HX = 60 * MM;
const HZ = 45 * MM;
/** The flange stands on the -X end of the plate, 10 mm thick. */
const FLANGE_X = (10 - 60) * MM;
const FLANGE_MID = (5 - 60) * MM;

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
const span = (p: number, a: number, b: number) => clamp01((p - a) / (b - a));
const smooth = (u: number) => u * u * (3 - 2 * u);
const { lerp } = THREE.MathUtils;

interface Stroke {
  line: Line2 | LineSegments2;
  mat: LineMaterial;
  length: number;
  from: number;
  to: number;
}

interface Drill {
  group: THREE.Group;
  ringMat: THREE.MeshBasicMaterial;
  surface: number;
  depth: number;
  start: number;
  fired: boolean;
}

const DRILL_DUR = 0.036;
const DRILL_STEP = 0.031;
const DRILL_HIT = 0.4;

function circlePoints(cx: number, cz: number, r: number, n: number, y: number) {
  const pts: number[] = [];
  for (let i = 0; i <= n; i++) {
    const a = (i / n) * Math.PI * 2;
    pts.push(cx + Math.cos(a) * r, y, cz + Math.sin(a) * r);
  }
  return pts;
}

/** A dimension line from a to b with arrow heads, as one polyline that backtracks over the tips. */
function dimensionPoints(ax: number, az: number, bx: number, bz: number, y: number) {
  const dx = bx - ax;
  const dz = bz - az;
  const len = Math.hypot(dx, dz);
  const ux = dx / len;
  const uz = dz / len;
  const nx = -uz;
  const nz = ux;
  const h = 0.07;
  const w = 0.028;
  return [
    ax + ux * h + nx * w, y, az + uz * h + nz * w,
    ax, y, az,
    ax + ux * h - nx * w, y, az + uz * h - nz * w,
    ax, y, az,
    bx, y, bz,
    bx - ux * h + nx * w, y, bz - uz * h + nz * w,
    bx, y, bz,
    bx - ux * h - nx * w, y, bz - uz * h - nz * w,
  ];
}

export function createCad(): Chapter {
  const group = new THREE.Group();
  group.name = "cad";
  const root = new THREE.Group();
  const part = new THREE.Group();
  const strokes: Stroke[] = [];
  const drills: Drill[] = [];
  const lineMaterials: LineMaterial[] = [];
  const lights: Array<{ light: THREE.Light; base: number }> = [];
  const clip = new THREE.Plane(new THREE.Vector3(0, -1, 0), PLANE_Y);
  const camPos = new THREE.Vector3();
  const target = new THREE.Vector3();
  const edgeUniforms = {
    uSweep: { value: -2 },
    uLit: { value: 1 },
    uBase: { value: 0 },
    uFront: { value: 0 },
  };

  let mesh: THREE.Mesh | null = null;
  let partMat: THREE.MeshPhysicalMaterial | null = null;
  let edgeMat: LineMaterial | null = null;
  let planeMat: THREE.MeshBasicMaterial | null = null;
  let cutMat: THREE.MeshBasicMaterial | null = null;
  let cutPlate: THREE.Mesh | null = null;
  let cutFlange: THREE.Mesh | null = null;
  let cutBoss: THREE.Mesh | null = null;
  let orbit = 0;
  let prev = 0;

  function lineMaterial(width: number) {
    const mat = new LineMaterial({
      color: ICE,
      linewidth: width,
      transparent: true,
      opacity: 0,
      depthWrite: false,
    });
    lineMaterials.push(mat);
    return mat;
  }

  function addStroke(points: number[], from: number, to: number, segments = false) {
    const mat = lineMaterial(2);
    mat.dashed = true;
    mat.dashSize = 0;
    mat.gapSize = 1e4;
    const line = segments
      ? new LineSegments2(new LineSegmentsGeometry().setPositions(points), mat)
      : new Line2(new LineGeometry().setPositions(points), mat);
    line.computeLineDistances();
    let length = 0;
    for (let i = 0; i < points.length - 3; i += segments ? 6 : 3) {
      length += Math.hypot(points[i + 3] - points[i], points[i + 4] - points[i + 1], points[i + 5] - points[i + 2]);
    }
    root.add(line);
    strokes.push({ line, mat, length, from, to });
  }

  function addDrill(x: number, z: number, r: number, surface: number, depth: number, start: number, quality: number) {
    const seg = quality > 0 ? 18 : 10;
    const tipLen = r * 2.4;
    const drill = new THREE.Group();
    const steel = new THREE.MeshStandardMaterial({ color: "#2b2e34", metalness: 0.85, roughness: 0.35 });
    const shaft = new THREE.Mesh(new THREE.CylinderGeometry(r, r, 1.2, seg), steel);
    shaft.position.y = tipLen + 0.6;
    const tip = new THREE.Mesh(
      new THREE.ConeGeometry(r, tipLen, seg),
      new THREE.MeshStandardMaterial({ color: "#0a1a22", emissive: ICE, emissiveIntensity: 2.2, roughness: 0.4 }),
    );
    tip.rotation.x = Math.PI;
    tip.position.y = tipLen / 2;
    drill.add(shaft, tip);
    drill.position.set(x, 1.6, z);
    drill.visible = false;
    const ringMat = new THREE.MeshBasicMaterial({
      color: ICE,
      transparent: true,
      opacity: 0,
      blending: THREE.AdditiveBlending,
      depthWrite: false,
    });
    const ring = new THREE.Mesh(new THREE.TorusGeometry(r + 0.014, 0.007, 6, quality > 0 ? 40 : 24), ringMat);
    ring.rotation.x = -Math.PI / 2;
    ring.position.set(x, surface + 0.004, z);
    part.add(drill, ring);
    drills.push({ group: drill, ringMat, surface, depth, start, fired: false });
  }

  function setResolution(ctx: ChapterContext) {
    for (const m of lineMaterials) m.resolution.set(ctx.viewport.width, ctx.viewport.height);
  }

  return {
    id: "cad",
    group,
    async init(ctx) {
      const gltf = await ctx.assets.gltf("/assets/3d/bracket.glb");
      mesh = fitPart(gltf, 1.8);
      partMat = new THREE.MeshPhysicalMaterial({
        color: "#c9ccd1",
        metalness: 0.9,
        roughness: 0.3,
        clearcoat: 0.2,
        side: THREE.DoubleSide,
        clippingPlanes: [clip],
        clipShadows: false,
        transparent: true,
        opacity: 1,
      });
      mesh.material = partMat;
      mesh.visible = false;
      part.add(mesh);
      part.position.y = PART_Y;

      // Edges: LineSegments2 with a sweep injected into LineMaterial. vEdgeX is the local x of
      // the edge vertex, so the glow can run from one end of the part to the other.
      const edges = new THREE.EdgesGeometry(mesh.geometry, 20);
      const edgeGeo = new LineSegmentsGeometry().fromEdgesGeometry(edges);
      edges.dispose();
      edgeMat = lineMaterial(1.6);
      edgeMat.opacity = 1;
      edgeMat.customProgramCacheKey = () => "cad-edge-sweep";
      edgeMat.onBeforeCompile = (shader) => {
        Object.assign(shader.uniforms, edgeUniforms);
        shader.vertexShader = shader.vertexShader
          .replace("attribute vec3 instanceStart;", "attribute vec3 instanceStart;\nvarying float vEdgeX;")
          .replace(
            "vec4 start = modelViewMatrix * vec4( instanceStart, 1.0 );",
            "vEdgeX = ( position.y < 0.5 ) ? instanceStart.x : instanceEnd.x;\nvec4 start = modelViewMatrix * vec4( instanceStart, 1.0 );",
          );
        shader.fragmentShader =
          "uniform float uSweep; uniform float uLit; uniform float uBase; uniform float uFront; varying float vEdgeX;\n" +
          shader.fragmentShader.replace(
            "vec4 diffuseColor = vec4( diffuse, alpha );",
            [
              "vec4 diffuseColor = vec4( diffuse, alpha );",
              "{",
              "  float d = vEdgeX - uSweep;",
              "  float lit = 1.0 - smoothstep( 0.0, 0.1, d );",
              "  float front = exp( -d * d * 300.0 ) * uFront;",
              "  float g = max( uBase, lit * uLit ) + front;",
              "  if ( g < 0.003 ) discard;",
              "  diffuseColor.a *= clamp( g, 0.0, 1.0 );",
              "  diffuseColor.rgb = mix( diffuseColor.rgb, vec3( 1.0 ), front * 0.6 );",
              "}",
            ].join("\n"),
          );
      };
      const edgeLines = new LineSegments2(edgeGeo, edgeMat);
      edgeLines.computeLineDistances();
      part.add(edgeLines);

      // Sketch plane and strokes, in root space at the plane height.
      planeMat = new THREE.MeshBasicMaterial({ color: ICE, transparent: true, opacity: 0, depthWrite: false });
      const plane = new THREE.Mesh(new THREE.PlaneGeometry(3.6, 2.8), planeMat);
      plane.rotation.x = -Math.PI / 2;
      plane.position.y = PLANE_Y - 0.01;
      root.add(plane);

      const y = PLANE_Y - 0.004;
      const n = ctx.quality > 0 ? 48 : 32;
      addStroke([-HX, y, -HZ, HX, y, -HZ, HX, y, HZ, -HX, y, HZ, -HX, y, -HZ], 0.0, 0.07);
      addStroke([FLANGE_X, y, -HZ, FLANGE_X, y, HZ], 0.05, 0.08);
      BOLTS.forEach(([x, z], i) => addStroke(circlePoints(x, z, 4.5 * MM, n, y), 0.06 + i * 0.015, 0.1 + i * 0.015));
      addStroke(circlePoints(BOSS[0], BOSS[1], 12 * MM, n, y), 0.1, 0.15);
      addStroke(circlePoints(BOSS[0], BOSS[1], 6 * MM, n, y), 0.13, 0.165);
      const zd = -HZ - 0.3;
      const xd = -HX - 0.3;
      addStroke(
        [-HX, y, -HZ - 0.04, -HX, y, zd - 0.06, HX, y, -HZ - 0.04, HX, y, zd - 0.06,
          -HX - 0.04, y, -HZ, xd - 0.06, y, -HZ, -HX - 0.04, y, HZ, xd - 0.06, y, HZ],
        0.14,
        0.17,
        true,
      );
      addStroke(dimensionPoints(-HX, zd, HX, zd, y), 0.15, 0.19);
      addStroke(dimensionPoints(xd, HZ, xd, -HZ, y), 0.16, 0.2);

      // Cut line: a glowing slab the size of whatever the clipping plane is passing through.
      cutMat = new THREE.MeshBasicMaterial({
        color: ICE,
        transparent: true,
        opacity: 0,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
      });
      const th = 0.016;
      cutPlate = new THREE.Mesh(new THREE.BoxGeometry(120 * MM + 0.006, th, 90 * MM + 0.006), cutMat);
      cutFlange = new THREE.Mesh(new THREE.BoxGeometry(10 * MM + 0.006, th, 90 * MM + 0.006), cutMat);
      cutFlange.position.x = FLANGE_MID;
      cutBoss = new THREE.Mesh(new THREE.CylinderGeometry(12 * MM + 0.004, 12 * MM + 0.004, th, 32), cutMat);
      cutBoss.position.set(BOSS[0], 0, BOSS[1]);
      cutPlate.visible = cutFlange.visible = cutBoss.visible = false;
      part.add(cutPlate, cutFlange, cutBoss);

      // Drills: four bolt holes through the plate, then the bore through the boss. Each owns a 0.036 window from 0.5.
      BOLTS.forEach(([x, z], i) =>
        addDrill(x, z, 4.5 * MM - 0.004, PLATE_TOP, 10 * MM + 0.08, 0.5 + i * DRILL_STEP, ctx.quality),
      );
      addDrill(BOSS[0], BOSS[1], 6 * MM - 0.006, BOSS_TOP, 30 * MM + 0.08, 0.5 + 4 * DRILL_STEP, ctx.quality);

      const key = new THREE.DirectionalLight("#ffffff", 1.4);
      key.position.set(3, 5, 2.5);
      const fill = new THREE.PointLight(ICE, 9, 12, 2);
      fill.position.set(-2.5, 1.6, 2.5);
      const rim = new THREE.PointLight("#ffffff", 3, 12, 2);
      rim.position.set(0.5, 2.8, -3);
      for (const light of [key, fill, rim]) lights.push({ light, base: light.intensity });

      root.add(part);
      root.position.x = 0.5;
      // Turn the part so the flange stands at the back left and the boss faces the camera.
      root.rotation.y = -2.3;
      group.add(root, key, fill, rim);
      setResolution(ctx);
    },
    enter(ctx) {
      ctx.setBackground("#07080a", { near: 6, far: 30 });
      ctx.floor.set({ color: ICE, intensity: 0.8, visible: true });
      ctx.setAccent(ICE);
    },
    exit() {},
    resize(ctx) {
      setResolution(ctx);
    },
    update(ctx, p, dt, t) {
      if (!mesh || !partMat || !edgeMat || !planeMat || !cutMat || !cutPlate || !cutFlange || !cutBoss) return;
      const fade = clamp01(Math.min(p / 0.06, (1 - p) / 0.06));
      const lift = smooth(span(p, 0.82, 0.94));
      const mobile = ctx.viewport.mobile;

      // Camera: slow orbit that speeds up once the part is free, pointer parallax on desktop.
      orbit += dt * (0.05 + lift * 0.14 + (ctx.pointer.coarse ? 0.08 : 0));
      const yaw = -0.5 + orbit + p * 0.35 + ctx.pointer.x * 0.22;
      const el = 0.4 - ctx.pointer.y * 0.1 + lift * 0.05;
      const dist = mobile ? 7.4 : 4.7;
      target.set(mobile ? 0.5 : 0.45, 0.78 + lift * 0.1, 0);
      camPos.set(Math.sin(yaw) * Math.cos(el) * dist, Math.sin(el) * dist, Math.cos(yaw) * Math.cos(el) * dist).add(target);
      ctx.camera.position.copy(camPos);
      ctx.camera.lookAt(target);

      for (const l of lights) l.light.intensity = l.base * fade;

      // Sketch: strokes draw on in their windows, pulse once drawn, fade in the last beat.
      const sketchAlpha = fade * (1 - span(p, 0.82, 0.9));
      planeMat.opacity = 0.03 * sketchAlpha;
      for (let i = 0; i < strokes.length; i++) {
        const s = strokes[i];
        const f = span(p, s.from, s.to);
        s.mat.dashSize = f * s.length + (f >= 1 ? 0.01 : 0);
        const pulse = f >= 1 ? 0.7 + 0.14 * Math.sin(t * 2.4 + i * 0.9) : 0.95;
        s.mat.opacity = f > 0 ? pulse * sketchAlpha : 0;
        s.line.visible = f > 0 && sketchAlpha > 0.002;
      }

      // Extrude: the clipping plane rises from the plate bottom to above the flange.
      const e = span(p, 0.2, 0.5);
      const cutLocal = e < 0.3 ? lerp(PLATE_BOTTOM, PLATE_TOP, e / 0.3) : lerp(PLATE_TOP, FLANGE_TOP + 0.03, (e - 0.3) / 0.7);
      const extruding = e > 0 && e < 1;
      mesh.visible = e > 0;
      clip.constant = e >= 1 ? 10 : PART_Y + cutLocal;
      partMat.opacity = fade;
      cutPlate.position.y = cutFlange.position.y = cutBoss.position.y = cutLocal;
      cutPlate.visible = extruding && cutLocal <= PLATE_TOP;
      cutFlange.visible = extruding && cutLocal > PLATE_TOP;
      cutBoss.visible = extruding && cutLocal > PLATE_TOP && cutLocal <= BOSS_TOP;
      cutMat.opacity = (0.32 + 0.1 * Math.sin(t * 9)) * fade;

      // Holes: each drill descends, hits at DRILL_HIT of its window, pushes through, retracts.
      const ringHold = 1 - span(p, 0.66, 0.74);
      for (const d of drills) {
        const u = span(p, d.start, d.start + DRILL_DUR);
        d.group.visible = u > 0 && u < 1;
        let tipY: number;
        if (u < DRILL_HIT) tipY = lerp(1.5, d.surface, smooth(u / DRILL_HIT));
        else if (u < 0.7) tipY = lerp(d.surface, d.surface - d.depth, (u - DRILL_HIT) / (0.7 - DRILL_HIT));
        else tipY = lerp(d.surface - d.depth, 1.5, smooth((u - 0.7) / 0.3));
        d.group.position.y = tipY;
        const k = (u - DRILL_HIT) / 0.09;
        const flash = Math.exp(-k * k);
        const stay = smooth(span(u, DRILL_HIT - 0.02, DRILL_HIT + 0.05)) * 0.22 * ringHold;
        d.ringMat.opacity = (flash + stay) * fade;
        const hit = d.start + DRILL_HIT * DRILL_DUR;
        if (Math.abs(p - prev) > 0.2) d.fired = p >= hit; // a jump (chapter activation) is a seek, not a scroll
        else if (!d.fired && prev < hit && p >= hit) {
          d.fired = true;
          ctx.cue("tick");
        } else if (d.fired && p < d.start) d.fired = false;
      }
      prev = p;

      // Edges: a sweep along x lights every edge, then settles to a faint constant glow.
      edgeUniforms.uSweep.value = lerp(-1.1, 1.1, span(p, 0.66, 0.79));
      edgeUniforms.uFront.value = 1 - span(p, 0.78, 0.8);
      edgeUniforms.uLit.value = 1 - 0.7 * span(p, 0.79, 0.84);
      edgeUniforms.uBase.value = 0.3 * span(p, 0.79, 0.84);
      edgeMat.opacity = fade;

      // Free: lift off the plane and tumble within bounded angles so reverse scroll returns home.
      part.position.y = PART_Y + lift * 0.3;
      part.rotation.set(lift * 0.4 * Math.sin(t * 0.37), lift * 0.8 * Math.sin(t * 0.21), lift * 0.3 * Math.sin(t * 0.29 + 1));
    },
    dispose() {
      group.traverse((o) => {
        const m = o as THREE.Mesh;
        if (m.geometry) m.geometry.dispose();
        const mat = m.material as THREE.Material | THREE.Material[] | undefined;
        if (Array.isArray(mat)) mat.forEach((x) => x.dispose());
        else mat?.dispose();
      });
    },
  };
}
