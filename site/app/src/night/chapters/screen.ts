/**
 * Screen chapter: a monitor comes down out of the dark, lands on the dot floor and the whole
 * picture shakes. Its screen is a real screenshot of the SolidDraft desktop app; in the second
 * beat it crossfades to a real screenshot of the browser build. Accent amber.
 * Everything is a function of progress except the landing effects, which fire on the crossing.
 */
import * as THREE from "three";
import { RoundedBoxGeometry } from "three/examples/jsm/geometries/RoundedBoxGeometry.js";

import type { Chapter, ChapterContext } from "../types";

const AMBER = "#f2b134";
/** Screenshots are 1600 by 1000, so the panel is 16:10. */
const PANEL = { w: 2.56, h: 1.6, d: 0.07, r: 0.04 };
const SCREEN = { w: 2.46, h: 1.5 };
const NECK = { w: 0.22, h: 0.42, d: 0.05 };
const FOOT = { w: 1.1, h: 0.035, d: 0.55 };
const MX = 0.45;
const MZ = 0;
const HANG_Y = 3.4;
const P_START = 0.08;
const P_LAND = 0.46;
const DUST_LIFE = 1.4;

const { clamp, lerp, smoothstep } = THREE.MathUtils;

function spriteTexture() {
  const c = document.createElement("canvas");
  c.width = c.height = 64;
  const g = c.getContext("2d")!;
  const grad = g.createRadialGradient(32, 32, 0, 32, 32, 32);
  grad.addColorStop(0, "rgba(255,255,255,1)");
  grad.addColorStop(0.35, "rgba(255,255,255,0.45)");
  grad.addColorStop(1, "rgba(255,255,255,0)");
  g.fillStyle = grad;
  g.fillRect(0, 0, 64, 64);
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

const RIM_DIM = new THREE.Color("#5a4a24");
const RIM_HOT = new THREE.Color("#ffd27a");

export function createScreen(): Chapter {
  const group = new THREE.Group();
  group.name = "screen";
  const monitor = new THREE.Group();
  const look = new THREE.Vector3();

  let bodyMat: THREE.MeshPhysicalMaterial | null = null;
  let rimMat: THREE.MeshBasicMaterial | null = null;
  let baseMat: THREE.MeshBasicMaterial | null = null;
  let topMat: THREE.MeshBasicMaterial | null = null;
  let offMat: THREE.MeshBasicMaterial | null = null;
  let shots: THREE.Texture[] = [];
  let dust: THREE.Points | null = null;
  let dustMat: THREE.PointsMaterial | null = null;
  let dustSeed: Float32Array | null = null;
  let sprite: THREE.Texture | null = null;
  let glow: THREE.PointLight | null = null;
  let key: THREE.DirectionalLight | null = null;
  let fill: THREE.DirectionalLight | null = null;
  let prev = -1;
  let landed = false;
  let landT = -100;
  let pulse = 0;

  return {
    id: "screen",
    group,

    async init(ctx: ChapterContext) {
      shots = await Promise.all([
        ctx.assets.texture("/assets/app/desktop-bracket-dark.webp"),
        ctx.assets.texture("/assets/app/browser-bracket.webp"),
      ]);
      const segments = ctx.quality === 0 ? 2 : 4;

      bodyMat = new THREE.MeshPhysicalMaterial({
        color: "#0c0d10",
        metalness: 0.6,
        roughness: 0.3,
        clearcoat: 1,
        clearcoatRoughness: 0.2,
        transparent: true,
      });
      const panel = new THREE.Mesh(new RoundedBoxGeometry(PANEL.w, PANEL.h, PANEL.d, segments, PANEL.r), bodyMat);
      panel.position.y = FOOT.h + NECK.h + PANEL.h / 2 - 0.05;
      rimMat = new THREE.MeshBasicMaterial({ color: RIM_DIM, side: THREE.BackSide, transparent: true, toneMapped: false });
      const rim = new THREE.Mesh(
        new RoundedBoxGeometry(PANEL.w + 0.014, PANEL.h + 0.014, PANEL.d + 0.014, segments, PANEL.r + 0.007),
        rimMat,
      );
      rim.position.copy(panel.position);
      const screenGeo = new THREE.PlaneGeometry(SCREEN.w, SCREEN.h);
      offMat = new THREE.MeshBasicMaterial({ color: "#05060a", transparent: true, toneMapped: false });
      const off = new THREE.Mesh(screenGeo, offMat);
      off.position.set(0, panel.position.y, PANEL.d / 2 + 0.001);
      baseMat = new THREE.MeshBasicMaterial({ map: shots[0], transparent: true, opacity: 0, toneMapped: false });
      const base = new THREE.Mesh(screenGeo, baseMat);
      base.position.set(0, panel.position.y, PANEL.d / 2 + 0.002);
      topMat = new THREE.MeshBasicMaterial({ map: shots[1], transparent: true, opacity: 0, depthWrite: false, toneMapped: false });
      const top = new THREE.Mesh(screenGeo, topMat);
      top.position.set(0, panel.position.y, PANEL.d / 2 + 0.003);
      const neck = new THREE.Mesh(new THREE.BoxGeometry(NECK.w, NECK.h + 0.1, NECK.d), bodyMat);
      neck.position.set(0, FOOT.h + NECK.h / 2, -PANEL.d / 2 - NECK.d / 2 + 0.01);
      const foot = new THREE.Mesh(new RoundedBoxGeometry(FOOT.w, FOOT.h, FOOT.d, segments, 0.015), bodyMat);
      foot.position.set(0, FOOT.h / 2, -0.12);
      monitor.add(panel, rim, off, base, top, neck, foot);
      monitor.position.set(MX, HANG_Y, MZ);

      glow = new THREE.PointLight(AMBER, 0, 7, 2);
      glow.position.set(0, panel.position.y, 1.2);
      monitor.add(glow);

      sprite = spriteTexture();
      const dustN = [300, 500, 800][ctx.quality];
      dustSeed = new Float32Array(dustN * 4);
      for (let i = 0; i < dustN; i++) {
        dustSeed[i * 4] = Math.random() * Math.PI * 2;
        dustSeed[i * 4 + 1] = 0.3 + Math.random() * 0.7;
        dustSeed[i * 4 + 2] = 1.0 + Math.random() * 2.6;
        dustSeed[i * 4 + 3] = 0.05 + Math.random() * 0.45;
      }
      const dustGeo = new THREE.BufferGeometry();
      dustGeo.setAttribute("position", new THREE.BufferAttribute(new Float32Array(dustN * 3), 3).setUsage(THREE.DynamicDrawUsage));
      dustMat = new THREE.PointsMaterial({
        map: sprite,
        color: "#5e4c3a",
        size: 0.4,
        transparent: true,
        opacity: 0,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
      });
      dust = new THREE.Points(dustGeo, dustMat);
      dust.frustumCulled = false;
      dust.visible = false;

      key = new THREE.DirectionalLight(AMBER, 2.0);
      key.position.set(5, 7, 3);
      key.target.position.set(MX, 1, MZ);
      fill = new THREE.DirectionalLight("#5b6f99", 0.7);
      fill.position.set(-5, 3, -4);
      fill.target.position.set(MX, 1, MZ);
      group.add(monitor, dust, key, key.target, fill, fill.target);
    },

    enter(ctx) {
      ctx.setBackground("#07080a", { near: 7, far: 34 });
      ctx.floor.set({ color: AMBER, intensity: 0.9, visible: true });
      ctx.setAccent(AMBER);
      prev = -1;
    },

    exit() {},

    update(ctx, p, dt, t) {
      if (!bodyMat || !rimMat || !baseMat || !topMat || !offMat || !dust || !dustMat || !dustSeed || !glow || !key || !fill) return;
      // Entering mid-chapter: before the touchdown, play the landing; after it, start landed and silent.
      if (prev < 0) prev = p < 0.55 ? 0 : p;
      if (p < 0.4) landed = false;
      const fade = clamp(Math.min(p / 0.06, (1 - p) / 0.06), 0, 1);
      const forward = p > prev && p - prev < 0.03;
      bodyMat.opacity = fade;
      bodyMat.transparent = fade < 0.999;
      pulse *= Math.exp(-dt * 4);

      // Descent and touchdown.
      const fall = clamp((p - P_START) / (P_LAND - P_START), 0, 1);
      const baseY = HANG_Y * (1 - fall * fall);
      if (p >= P_LAND && !landed) {
        landed = true;
        if (prev < P_LAND) {
          landT = t;
          ctx.shake(0.9);
          ctx.floor.ripple(MX, MZ, 0.9);
          ctx.cue("thud");
          pulse = 1;
        }
      }
      const age = t - landT;
      const squash = age < 0.3 ? Math.sin((age / 0.3) * Math.PI) : 0;
      const swing = (1 - fall) * 0.06 * Math.sin(t * 1.3);
      monitor.position.set(MX, baseY, MZ);
      monitor.rotation.set(swing * 0.5, -0.12 + (1 - fall) * 0.25 + Math.sin(t * 0.37) * 0.02 * fall, swing);
      monitor.scale.set(1 + 0.012 * squash, 1 - 0.025 * squash, 1 + 0.012 * squash);

      // The screen switches on at touchdown, then crossfades desktop to browser in the second beat.
      const on = smoothstep(p, P_LAND, P_LAND + 0.05);
      const swap = smoothstep(p, 0.62, 0.74);
      if (forward && prev < 0.62 && p >= 0.62) ctx.cue("tick");
      prev = p;
      offMat.opacity = fade;
      baseMat.opacity = on * fade;
      topMat.opacity = swap * on * fade;
      rimMat.opacity = fade;
      rimMat.color.copy(RIM_DIM).lerp(RIM_HOT, pulse + on * 0.25);
      glow.intensity = (6 * on + 20 * pulse) * fade;

      // Dust burst: a ring from under the foot, rising a little, gone in 1.4 s.
      if (age < DUST_LIFE) {
        dust.visible = true;
        const dp = dust.geometry.attributes.position.array as Float32Array;
        const n = dustSeed.length / 4;
        const spread = (1 - Math.exp(-age * 2.2)) / 2.2;
        const lift = 1 - Math.exp(-age * 1.5);
        for (let i = 0; i < n; i++) {
          const ang = dustSeed[i * 4];
          const s = dustSeed[i * 4 + 1];
          const r = dustSeed[i * 4 + 2] * spread;
          dp[i * 3] = MX + Math.cos(ang) * (FOOT.w * 0.5 * s + r);
          dp[i * 3 + 1] = 0.04 + dustSeed[i * 4 + 3] * lift;
          dp[i * 3 + 2] = MZ + Math.sin(ang) * (FOOT.d * 0.5 * s + r * 0.8);
        }
        dust.geometry.attributes.position.needsUpdate = true;
        dustMat.opacity = Math.pow(1 - age / DUST_LIFE, 1.4) * 0.3 * fade;
        dustMat.size = 0.4 + age * 0.5;
      } else {
        dust.visible = false;
      }

      key.intensity = 2.0 * fade;
      fill.intensity = 0.7 * fade;

      // Camera: low while the monitor hangs, settles level on the screen, a few degrees of pointer.
      const after = smoothstep(p, 0.5, 0.8);
      const mobile = ctx.viewport.mobile;
      const fov = ctx.camera.fov * THREE.MathUtils.DEG2RAD;
      const fitW = lerp(3.6, 3.2, after);
      const dist = Math.max(4.2, fitW / (2 * Math.tan(fov / 2) * ctx.viewport.aspect));
      const px = ctx.pointer.coarse ? Math.sin(t * 0.3) * 0.5 : ctx.pointer.x;
      const py = ctx.pointer.coarse ? 0 : ctx.pointer.y;
      const yaw = 0.22 - after * 0.1 + px * 0.16 + Math.sin(t * 0.3) * 0.02;
      const screenY = FOOT.h + NECK.h + PANEL.h / 2 - 0.05;
      const camY = lerp(0.9, screenY + 0.25, after) + baseY * 0.35 - py * 0.3;
      look.set(MX + (mobile ? 0 : -0.35), lerp(0.7, screenY - 0.05, after) + baseY * 0.5, MZ);
      ctx.camera.position.set(look.x + Math.sin(yaw) * dist, camY, look.z + Math.cos(yaw) * dist);
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
      for (const tex of shots) tex.dispose();
      sprite?.dispose();
    },
  };
}
