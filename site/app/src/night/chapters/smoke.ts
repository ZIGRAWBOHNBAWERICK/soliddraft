/**
 * Chapter `smoke`: the projects room. Dark again, with slow domain-warped fbm smoke on a
 * full-screen quad behind the work cards and the contact block, plus a few embers drifting up.
 * Dim on purpose: white text and cards sit over it. Runs to the end of the page, so it fades in
 * only.
 */
import * as THREE from "three";

import type { Chapter } from "../types";

const BG = "#07080a";

const quadVert = /* glsl */ `
  varying vec2 vUv;
  void main() {
    vUv = uv;
    gl_Position = vec4(position.xy, 0.0, 1.0);
  }
`;

const quadFrag = /* glsl */ `
  precision highp float;
  uniform float uTime;
  uniform float uProgress;
  uniform float uFade;
  uniform float uAspect;
  uniform vec2 uPointer;
  varying vec2 vUv;

  float hash(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
  }
  float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash(i);
    float b = hash(i + vec2(1.0, 0.0));
    float c = hash(i + vec2(0.0, 1.0));
    float d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
  }
  float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    mat2 r = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < OCTAVES; i++) {
      v += a * noise(p);
      p = r * p * 2.03 + 1.7;
      a *= 0.5;
    }
    return v;
  }

  void main() {
    vec2 p = (vUv - 0.5) * vec2(uAspect, 1.0) * 1.8;
    float t = uTime * 0.06 + uProgress * 1.4;
    // Two warp fields moving in different directions, nudged by the pointer.
    vec2 warp = vec2(
      fbm(p * 1.1 + vec2(t * 0.7, -t * 0.4)),
      fbm(p * 1.1 + vec2(-t * 0.5, t * 0.6) + 3.1)
    ) + uPointer * 0.18;
    float a = fbm(p + 1.6 * warp + vec2(t, t * 0.3));
    float b = fbm(p * 1.7 - 1.2 * warp.yx + vec2(-t * 0.8, t * 0.5) + 7.0);
    float s = smoothstep(0.3, 0.72, a * 0.65 + b * 0.45);
    // Near black through deep violet to a lighter violet, with oxide only in the brightest wisps.
    vec3 col = mix(vec3(0.02, 0.02, 0.03), vec3(0.165, 0.122, 0.322), pow(s, 0.8));
    col = mix(col, vec3(0.4, 0.3, 0.64), smoothstep(0.45, 1.0, s) * 0.75);
    col = mix(col, vec3(1.0, 0.357, 0.122), smoothstep(0.86, 1.0, s) * 0.14);
    float vig = smoothstep(1.5, 0.35, length((vUv - 0.5) * vec2(uAspect, 1.0) * 1.5));
    col *= vig;
    col = mix(vec3(0.027, 0.031, 0.039), col, uFade);
    gl_FragColor = vec4(col, 1.0);
  }
`;

const emberVert = /* glsl */ `
  attribute float aSeed;
  uniform float uTime;
  uniform float uFade;
  uniform float uDpr;
  varying float vAlpha;
  varying vec3 vColor;
  void main() {
    float life = fract(uTime / (22.0 + aSeed * 16.0) + aSeed);
    vec3 p = position;
    p.y = -1.0 + life * 6.0;
    p.x += sin(uTime * 0.25 + aSeed * 40.0) * 0.5;
    vColor = aSeed > 0.72 ? vec3(1.0, 0.357, 0.122) : vec3(0.46, 0.38, 0.84);
    vAlpha = sin(life * 3.14159) * (0.5 + 0.4 * fract(aSeed * 7.0)) * uFade;
    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    gl_PointSize = (3.0 + fract(aSeed * 13.0) * 4.0) * uDpr * (6.0 / -mv.z);
    gl_Position = projectionMatrix * mv;
  }
`;

const emberFrag = /* glsl */ `
  precision highp float;
  varying float vAlpha;
  varying vec3 vColor;
  void main() {
    float d = length(gl_PointCoord - 0.5);
    float a = smoothstep(0.5, 0.08, d) * vAlpha;
    gl_FragColor = vec4(vColor * a, a);
  }
`;

export function createSmoke(): Chapter {
  const group = new THREE.Group();
  group.name = "smoke";
  const uniforms = {
    uTime: { value: 0 },
    uProgress: { value: 0 },
    uFade: { value: 0 },
    uAspect: { value: 1 },
    uPointer: { value: new THREE.Vector2() },
    uDpr: { value: 1 },
  };
  const quadGeo = new THREE.PlaneGeometry(2, 2);
  const quadMat = new THREE.ShaderMaterial({ vertexShader: quadVert, fragmentShader: quadFrag, uniforms, depthTest: false, depthWrite: false });
  const emberGeo = new THREE.BufferGeometry();
  const emberMat = new THREE.ShaderMaterial({
    vertexShader: emberVert,
    fragmentShader: emberFrag,
    uniforms,
    transparent: true,
    depthTest: false,
    depthWrite: false,
    blending: THREE.AdditiveBlending,
  });

  return {
    id: "smoke",
    group,
    init(ctx) {
      quadMat.defines = { OCTAVES: ctx.quality === 0 ? 3 : 4 };
      const quad = new THREE.Mesh(quadGeo, quadMat);
      quad.frustumCulled = false;
      quad.renderOrder = 10;

      const n = ctx.quality === 0 ? 20 : 40;
      const pos = new Float32Array(n * 3);
      const seed = new Float32Array(n);
      for (let i = 0; i < n; i++) {
        pos[i * 3] = (Math.random() - 0.5) * 9;
        pos[i * 3 + 2] = Math.random() * 4;
        seed[i] = Math.random();
      }
      emberGeo.setAttribute("position", new THREE.BufferAttribute(pos, 3));
      emberGeo.setAttribute("aSeed", new THREE.BufferAttribute(seed, 1));
      const embers = new THREE.Points(emberGeo, emberMat);
      embers.frustumCulled = false;
      embers.renderOrder = 11;
      group.add(quad, embers);
    },
    enter(ctx) {
      // Soft full-screen noise: half resolution is invisible here and halves the fill cost.
      ctx.setRenderScale(0.5);
      ctx.setBackground(BG);
      ctx.floor.set({ visible: false });
      ctx.setAccent("#7660d6");
    },
    exit() {},
    update(ctx, progress, _dt, t) {
      uniforms.uTime.value = t;
      uniforms.uProgress.value = progress;
      uniforms.uFade.value = THREE.MathUtils.clamp(progress / 0.06, 0, 1);
      uniforms.uAspect.value = ctx.viewport.aspect;
      uniforms.uDpr.value = ctx.viewport.dpr;
      const px = ctx.pointer.coarse ? Math.sin(t * 0.2) * 0.4 : ctx.pointer.x;
      const py = ctx.pointer.coarse ? Math.cos(t * 0.15) * 0.3 : ctx.pointer.y;
      uniforms.uPointer.value.set(px, -py);
      ctx.camera.position.set(0, 1.6, 6);
      ctx.camera.lookAt(0, 1.6, 0);
    },
    dispose() {
      quadGeo.dispose();
      quadMat.dispose();
      emberGeo.dispose();
      emberMat.dispose();
    },
  };
}
