/**
 * Chapter `light`: the white room of parts. The page turns bone here. Eight real SolidDraft parts
 * drift as a quiet field in the right two thirds of the frame while the camera dollies through
 * it; the left third stays clear for the copy, with only small distant pieces behind it.
 */
import * as THREE from "three";

import { fitPart } from "../assets";
import type { Chapter } from "../types";

const BG = "#f2f0ea";

/** Resting positions. The bracket is the hero, nearest and largest, centre right. Every part is a real SolidDraft export. */
const FIELD = [
  { name: "bracket", size: 1.5, x: 1.9, y: 1.9, z: -2.5 },
  { name: "collar", size: 1.2, x: 3.6, y: 1.4, z: -4.5 },
  { name: "lattice", size: 1.0, x: 0.3, y: 3.3, z: -6.5 },
  { name: "plate", size: 1.3, x: 3.8, y: 2.9, z: -9 },
  { name: "boss", size: 1.0, x: 2.0, y: 0.9, z: -12 },
  { name: "tray", size: 1.1, x: -0.6, y: 2.4, z: -13.5 },
  { name: "gyroid", size: 1.0, x: 4.4, y: 0.8, z: -7.5 },
  { name: "fillet", size: 0.9, x: -0.2, y: 1.2, z: -10.5 },
];

interface Drifter {
  /** The bracket: yaw so the camera looks into the open corner of the L, and swing instead of spinning. */
  hero?: boolean;
  x: number;
  y: number;
  z: number;
  phase: number;
  spin: number;
  bob: number;
  scale: number;
}

/** Seeded so every screenshot shows the same room. */
function mulberry(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function drifter(rand: () => number, x: number, y: number, z: number): Drifter {
  return {
    x,
    y,
    z,
    phase: rand() * Math.PI * 2,
    spin: 0.07 + rand() * 0.12,
    bob: 0.08 + rand() * 0.12,
    scale: 0.6 + rand() * 1.3,
  };
}

/** Slow bob and turn; `xs` squeezes the field toward the centre on narrow screens. */
function place(o: THREE.Object3D, d: Drifter, t: number, xs: number) {
  o.position.set(d.x * xs, d.y + Math.sin(t * 0.45 + d.phase) * d.bob, d.z);
  if (d.hero) {
    o.rotation.set(-0.32 + Math.sin(t * 0.23) * 0.08, Math.PI + Math.sin(t * 0.17) * 0.5, Math.sin(t * 0.13) * 0.06);
    return;
  }
  o.rotation.set(Math.sin(t * 0.21 + d.phase) * 0.25, t * d.spin + d.phase, Math.cos(t * 0.17 + d.phase) * 0.15);
}

export function createLight(): Chapter {
  const group = new THREE.Group();
  group.name = "light";
  const material = new THREE.MeshStandardMaterial({ color: "#f4f2ec", roughness: 0.85, metalness: 0 });
  const parts: Array<{ mesh: THREE.Mesh; d: Drifter }> = [];
  const chips: Array<{ inst: THREE.InstancedMesh; d: Drifter[] }> = [];
  const dummy = new THREE.Object3D();
  const look = new THREE.Vector3();

  return {
    id: "light",
    group,
    async init(ctx) {
      const rand = mulberry(7);
      const gltfs = await Promise.all(FIELD.map((f) => ctx.assets.gltf(`/assets/3d/${f.name}.glb`)));
      gltfs.forEach((gltf, i) => {
        const f = FIELD[i];
        const mesh = fitPart(gltf, f.size);
        mesh.material = material;
        group.add(mesh);
        parts.push({ mesh, d: { ...drifter(rand, f.x, f.y, f.z), hero: i === 0 } });
      });

      // Small chips further out so the room feels populated. Anything on the copy's side sits far back.
      const count = ctx.quality === 0 ? 9 : 18;
      const nBox = Math.ceil(count / 2);
      const geoms = [new THREE.BoxGeometry(0.22, 0.14, 0.3), new THREE.CylinderGeometry(0.12, 0.12, 0.2, 14)];
      [nBox, count - nBox].forEach((n, gi) => {
        const inst = new THREE.InstancedMesh(geoms[gi], material, n);
        inst.frustumCulled = false;
        const d: Drifter[] = [];
        for (let i = 0; i < n; i++) {
          const x = -2.5 + rand() * 11;
          const y = 0.3 + rand() * 5;
          let z = -17 + rand() * 16;
          if (x < 0.5) z = Math.min(z, -11);
          d.push(drifter(rand, x, y, z));
        }
        chips.push({ inst, d });
        group.add(inst);
      });

      const hemi = new THREE.HemisphereLight("#f6f3ec", "#857d72", 0.55);
      const key = new THREE.DirectionalLight("#ffffff", 2.4);
      key.position.set(4, 7, 5);
      key.target.position.set(1, 2, -5);
      group.add(hemi, key, key.target);
    },
    enter(ctx) {
      ctx.setBackground(BG, { near: 5, far: 26 });
      ctx.floor.set({ color: "#0b0c0d", intensity: 0.35, visible: true });
      ctx.setAccent("#ff5b1f");
    },
    exit() {},
    update(ctx, progress, _dt, t) {
      const fade = THREE.MathUtils.clamp(Math.min(progress / 0.06, (1 - progress) / 0.06), 0, 1);
      material.opacity = fade;
      material.transparent = fade < 0.999;

      const xs = ctx.viewport.mobile ? 0.4 : 1;
      for (const p of parts) place(p.mesh, p.d, t, xs);
      for (const c of chips) {
        for (let i = 0; i < c.d.length; i++) {
          place(dummy, c.d[i], t, xs);
          dummy.scale.setScalar(c.d[i].scale);
          dummy.updateMatrix();
          c.inst.setMatrixAt(i, dummy.matrix);
        }
        c.inst.instanceMatrix.needsUpdate = true;
      }

      const px = ctx.pointer.coarse ? Math.sin(t * 0.23) * 0.5 : ctx.pointer.x;
      const py = ctx.pointer.coarse ? Math.sin(t * 0.17) * 0.3 : ctx.pointer.y;
      const z = 6 - progress * 4;
      ctx.camera.position.set(px * 0.4, 1.7 - py * 0.25, z);
      look.set((ctx.viewport.mobile ? 0.3 : 0.3) + px * 0.5, 2.1 - py * 0.35, z - 8);
      ctx.camera.lookAt(look);
    },
    dispose() {
      group.traverse((o) => {
        const m = o as THREE.Mesh;
        if (m.geometry) m.geometry.dispose();
        if ((m as THREE.InstancedMesh).isInstancedMesh) (m as THREE.InstancedMesh).dispose();
      });
      material.dispose();
    },
  };
}
