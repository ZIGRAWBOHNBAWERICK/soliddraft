/**
 * Drawing chapter: the bracket becomes its drawing. The real ISO A3 sheet SolidDraft made of
 * the part (site/parts, `soliddraft draw`) stands behind the part and is revealed by a violet
 * scan line while the part turns to its front view (0 to 0.5). Then the part lifts away, the
 * sheet comes forward and fans into its three exports (0.6 to 1). Accent violet.
 */
import * as THREE from "three";

import { fitPart } from "../assets";
import type { Chapter, ChapterContext } from "../types";

const VIOLET = "#7660d6";
/** The sheet is A3 landscape: 420 by 297 mm. */
const SHEET_W = 2.7;
const SHEET_H = SHEET_W * (297 / 420);
/** The sheet and the part keep to the upper right two thirds; the caption owns the bottom left. */
const SHEET = new THREE.Vector3(1.15, 1.95, -0.9);
const PART = new THREE.Vector3(1.35, 1.25, 0.35);
const COPIES = 3;

const { clamp, lerp, smoothstep } = THREE.MathUtils;

export function createDraw(): Chapter {
  const group = new THREE.Group();
  group.name = "draw";

  const sheetGroup = new THREE.Group();
  const partGroup = new THREE.Group();
  const copies: THREE.Mesh[] = [];
  const copyMats: THREE.MeshBasicMaterial[] = [];
  /** Keeps x <= constant in sheet space: the reveal wipes from left to right. */
  const wipe = new THREE.Plane(new THREE.Vector3(-1, 0, 0), 0);
  const wipeWorld = new THREE.Plane();
  const look = new THREE.Vector3();

  let partMesh: THREE.Mesh | null = null;
  let partMat: THREE.MeshStandardMaterial | null = null;
  let sheetMat: THREE.MeshBasicMaterial | null = null;
  let scanMat: THREE.MeshBasicMaterial | null = null;
  let scan: THREE.Mesh | null = null;
  let glow: THREE.PointLight | null = null;
  let key: THREE.DirectionalLight | null = null;
  let fill: THREE.PointLight | null = null;
  let sheetTex: THREE.Texture | null = null;
  let prev = -1;

  return {
    id: "draw",
    group,

    async init(ctx: ChapterContext) {
      const [gltf, tex] = await Promise.all([
        ctx.assets.gltf("/assets/3d/bracket.glb"),
        ctx.assets.texture("/assets/drawing/bracket-sheet.webp"),
      ]);
      sheetTex = tex;
      partMesh = fitPart(gltf, 1.05);
      partMat = new THREE.MeshStandardMaterial({ color: "#e4e1d9", roughness: 0.55, metalness: 0.1, transparent: true });
      partMesh.material = partMat;
      partGroup.add(partMesh);
      partGroup.position.copy(PART);

      // The sheet, with the wipe as a local clipping plane. Not tone mapped: paper stays paper.
      const sheetGeo = new THREE.PlaneGeometry(SHEET_W, SHEET_H);
      sheetMat = new THREE.MeshBasicMaterial({
        map: tex,
        transparent: true,
        opacity: 0,
        toneMapped: false,
        clippingPlanes: [wipeWorld],
        side: THREE.DoubleSide,
      });
      const sheet = new THREE.Mesh(sheetGeo, sheetMat);
      sheetGroup.add(sheet);
      // Export copies that fan out behind the sheet in the second beat.
      for (let i = 0; i < COPIES; i++) {
        const m = new THREE.MeshBasicMaterial({
          map: tex,
          transparent: true,
          opacity: 0,
          toneMapped: false,
          side: THREE.DoubleSide,
          depthWrite: false,
        });
        const c = new THREE.Mesh(sheetGeo, m);
        c.visible = false;
        copies.push(c);
        copyMats.push(m);
        sheetGroup.add(c);
      }
      sheetGroup.position.copy(SHEET);

      // Scan line: a thin additive slab the height of the sheet, moving with the wipe.
      scanMat = new THREE.MeshBasicMaterial({
        color: VIOLET,
        transparent: true,
        opacity: 0,
        blending: THREE.AdditiveBlending,
        depthWrite: false,
        side: THREE.DoubleSide,
      });
      scan = new THREE.Mesh(new THREE.PlaneGeometry(0.03, SHEET_H + 0.08), scanMat);
      scan.position.z = 0.01;
      sheetGroup.add(scan);
      glow = new THREE.PointLight(VIOLET, 0, 6, 2);
      glow.position.z = 0.5;
      sheetGroup.add(glow);

      key = new THREE.DirectionalLight("#ffffff", 1.8);
      key.position.set(3, 5, 4);
      fill = new THREE.PointLight(VIOLET, 10, 12, 2);
      fill.position.set(-2.5, 1.2, 2.5);
      group.add(sheetGroup, partGroup, key, fill);
    },

    enter(ctx) {
      ctx.setBackground("#07080a", { near: 6, far: 30 });
      ctx.floor.set({ color: VIOLET, intensity: 0.9, visible: true });
      ctx.setAccent(VIOLET);
      prev = -1;
    },

    exit() {},

    update(ctx, p, _dt, t) {
      if (!partMesh || !partMat || !sheetMat || !scanMat || !scan || !glow || !key || !fill) return;
      const fade = clamp(Math.min(p / 0.06, (1 - p) / 0.06), 0, 1);
      const reveal = smoothstep(p, 0.1, 0.46);
      const front = smoothstep(p, 0.14, 0.4);
      const away = smoothstep(p, 0.54, 0.7);
      const fan = smoothstep(p, 0.7, 0.86);
      const forward = prev >= 0 && p > prev && p - prev < 0.03;
      if (forward && prev < 0.1 && p >= 0.1) ctx.cue("whoosh");
      if (forward && prev < 0.72 && p >= 0.72) ctx.cue("chime");
      prev = p;
      const { pointer } = ctx;
      const px = pointer.coarse ? Math.sin(t * 0.3) * 0.5 : pointer.x;
      const py = pointer.coarse ? 0 : pointer.y;

      // Part: an isometric pose that turns to the drawing's front view as the scan passes, then
      // sinks and drifts off to the right while the sheet takes over.
      partGroup.position.set(PART.x + away * 1.8, PART.y - away * 1.2 + Math.sin(t * 0.9) * 0.012, PART.z + away * 0.4);
      partGroup.rotation.set(lerp(-0.35, 0, front) + Math.sin(t * 0.5) * 0.015, lerp(0.75, Math.PI / 2, front) + away * 1.2, 0);
      partGroup.scale.setScalar(lerp(0.85, 1, fade) * (1 - 0.5 * away));
      partMat.opacity = fade * (1 - away);
      partMesh.visible = partMat.opacity > 0.002;

      // Sheet: upright behind the part, wiped on left to right, then tilted toward the camera.
      const tilt = smoothstep(p, 0.56, 0.8);
      sheetGroup.position.set(SHEET.x - tilt * 0.05, SHEET.y - tilt * 0.12, SHEET.z + tilt * 0.35);
      sheetGroup.rotation.set(-tilt * 0.28 + py * 0.04, px * 0.08 + Math.sin(t * 0.35) * 0.012, 0);
      sheetGroup.updateMatrixWorld();
      const edge = lerp(-SHEET_W / 2 - 0.05, SHEET_W / 2 + 0.05, reveal);
      wipe.normal.set(-1, 0, 0);
      wipe.constant = edge;
      wipeWorld.copy(wipe).applyMatrix4(sheetGroup.matrixWorld);
      sheetMat.opacity = fade * smoothstep(p, 0.08, 0.12);
      scan.position.x = edge;
      const scanning = reveal > 0 && reveal < 1 ? 1 : 0;
      scanMat.opacity = fade * scanning * (0.7 + 0.3 * Math.sin(t * 12));
      scan.visible = scanMat.opacity > 0.002;
      glow.position.x = edge;
      glow.intensity = 16 * fade * scanning;

      // Copies: SVG, PDF and DXF fan out behind the sheet, each a little further back.
      for (let i = 0; i < COPIES; i++) {
        const k = (i + 1) / COPIES;
        const c = copies[i];
        c.visible = fan > 0.002;
        c.position.set(0.26 * k * fan, -0.14 * k * fan, -0.12 * k * fan);
        c.rotation.z = -0.045 * k * fan;
        copyMats[i].opacity = fade * fan * (0.55 - 0.14 * i);
      }

      key.intensity = 1.8 * fade;
      fill.intensity = 10 * fade;

      // Camera: fit the sheet, dolly in a little for the second beat, a few degrees of pointer.
      const fov = ctx.camera.fov * THREE.MathUtils.DEG2RAD;
      const fitW = lerp(5.2, 4.8, tilt);
      const dist = Math.max(4.6, fitW / (2 * Math.tan(fov / 2) * ctx.viewport.aspect));
      const lookX = ctx.viewport.mobile ? 1.0 : 0.75;
      const yaw = px * 0.12 + Math.sin(t * 0.4) * 0.02;
      ctx.camera.position.set(lookX + Math.sin(yaw) * dist, 1.75 + py * 0.22 + Math.sin(t * 0.2) * 0.02, SHEET.z + Math.cos(yaw) * dist);
      look.set(lookX, 1.55 - tilt * 0.05, SHEET.z);
      ctx.camera.lookAt(look);
    },

    dispose() {
      group.traverse((o) => {
        const m = o as THREE.Mesh;
        if (m.geometry) m.geometry.dispose();
        const mat = m.material as THREE.Material | THREE.Material[] | undefined;
        if (Array.isArray(mat)) mat.forEach((x) => x.dispose());
        else mat?.dispose();
      });
      sheetTex?.dispose();
    },
  };
}
