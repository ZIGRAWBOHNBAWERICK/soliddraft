/**
 * Hero: the night landscape you drop into. A displaced plane with ridged noise, a sky dome with
 * a low warm glow behind the ridges, valley lights and stars as point sprites. The pointer yaws
 * the view a few degrees; scrolling lifts the camera, thickens the fog and lets the lights go.
 */
import * as THREE from "three";

import type { Chapter, ChapterContext } from "../types";

const DEG = Math.PI / 180;

// Height field shared by the terrain and the lights, so a light always sits on the ground under
// it. The ridged noise is baked once into a texture that tiles along z; the drift is a UV scroll.
const heightGlsl = /* glsl */ `
  uniform sampler2D uHeight;
  uniform float uTime;
  uniform float uSink;
  float field(vec2 w) {
    return texture2D(uHeight, vec2((w.x + 130.0) / 260.0, (w.y - uTime * 0.12) / 340.0)).r;
  }
  // Height in world units at a world xz. The shape scrolls toward the camera; the envelope stays put.
  float terrainH(vec2 w) {
    float far = smoothstep(-4.0, 85.0, -w.y);
    float amp = mix(4.5, 30.0, far);
    float xv = 7.0 + 5.0 * sin(w.y * 0.045 + 0.8);
    float valley = 1.0 - 0.85 * exp(-pow((w.x - xv) / 12.0, 2.0)) * (1.0 - far * 0.75);
    return field(w) * amp * valley;
  }
`;

// 3D simplex noise (Gustavson), used once at init to bake the heightmap.
const GRAD = [1, 1, 0, -1, 1, 0, 1, -1, 0, -1, -1, 0, 1, 0, 1, -1, 0, 1, 1, 0, -1, -1, 0, -1, 0, 1, 1, 0, -1, 1, 0, 1, -1, 0, -1, -1];
const PERM = new Uint8Array(512);
{
  let seed = 1337;
  const rand = () => ((seed = (seed * 16807) % 2147483647) - 1) / 2147483646;
  const p = Array.from({ length: 256 }, (_, i) => i);
  for (let i = 255; i > 0; i--) {
    const j = Math.floor(rand() * (i + 1));
    [p[i], p[j]] = [p[j], p[i]];
  }
  for (let i = 0; i < 512; i++) PERM[i] = p[i & 255];
}
function snoise3(x: number, y: number, z: number) {
  const F3 = 1 / 3;
  const G3 = 1 / 6;
  const s = (x + y + z) * F3;
  const i = Math.floor(x + s);
  const j = Math.floor(y + s);
  const k = Math.floor(z + s);
  const t = (i + j + k) * G3;
  const x0 = x - (i - t);
  const y0 = y - (j - t);
  const z0 = z - (k - t);
  let i1: number, j1: number, k1: number, i2: number, j2: number, k2: number;
  if (x0 >= y0) {
    if (y0 >= z0) [i1, j1, k1, i2, j2, k2] = [1, 0, 0, 1, 1, 0];
    else if (x0 >= z0) [i1, j1, k1, i2, j2, k2] = [1, 0, 0, 1, 0, 1];
    else [i1, j1, k1, i2, j2, k2] = [0, 0, 1, 1, 0, 1];
  } else if (y0 < z0) [i1, j1, k1, i2, j2, k2] = [0, 0, 1, 0, 1, 1];
  else if (x0 < z0) [i1, j1, k1, i2, j2, k2] = [0, 1, 0, 0, 1, 1];
  else [i1, j1, k1, i2, j2, k2] = [0, 1, 0, 1, 1, 0];
  const ii = i & 255;
  const jj = j & 255;
  const kk = k & 255;
  let n = 0;
  const corner = (dx: number, dy: number, dz: number, gi: number) => {
    let tt = 0.6 - dx * dx - dy * dy - dz * dz;
    if (tt < 0) return 0;
    tt *= tt;
    const g = (gi % 12) * 3;
    return tt * tt * (GRAD[g] * dx + GRAD[g + 1] * dy + GRAD[g + 2] * dz);
  };
  n += corner(x0, y0, z0, PERM[ii + PERM[jj + PERM[kk]]]);
  n += corner(x0 - i1 + G3, y0 - j1 + G3, z0 - k1 + G3, PERM[ii + i1 + PERM[jj + j1 + PERM[kk + k1]]]);
  n += corner(x0 - i2 + 2 * G3, y0 - j2 + 2 * G3, z0 - k2 + 2 * G3, PERM[ii + i2 + PERM[jj + j2 + PERM[kk + k2]]]);
  n += corner(x0 - 1 + 3 * G3, y0 - 1 + 3 * G3, z0 - 1 + 3 * G3, PERM[ii + 1 + PERM[jj + 1 + PERM[kk + 1]]]);
  return 32 * n;
}

// Ridged multifractal (sharp crests, soft valleys) sampled on a cylinder so it tiles along z.
function bakeHeightmap(w: number, h: number, sizeX: number, sizeZ: number) {
  const data = new Uint16Array(w * h);
  const R = sizeZ / (2 * Math.PI);
  const ridged = (x: number, cy: number, cz: number, f0: number) => {
    let sum = 0;
    let a = 0.55;
    let f = f0;
    let wgt = 1;
    for (let o = 0; o < 5; o++) {
      let n = 1 - Math.abs(snoise3(x * f, cy * f, cz * f));
      n *= n;
      n *= wgt;
      wgt = Math.min(1, Math.max(0, n * 1.6));
      sum += n * a;
      f *= 2.05;
      a *= 0.48;
    }
    return sum;
  };
  for (let j = 0; j < h; j++) {
    const th = (j / h) * Math.PI * 2;
    const cy = Math.cos(th) * R;
    const cz = Math.sin(th) * R;
    for (let i = 0; i < w; i++) {
      const x = (i / w - 0.5) * sizeX;
      const r = ridged(x, cy, cz, 0.017) + ridged(x + 400, cy, cz, 0.06) * 0.12;
      const macro = 0.7 + 0.4 * snoise3(x * 0.009 + 3.1, cy * 0.009, cz * 0.009);
      data[j * w + i] = THREE.DataUtils.toHalfFloat(r * macro);
    }
  }
  const tex = new THREE.DataTexture(data, w, h, THREE.RedFormat, THREE.HalfFloatType);
  tex.wrapS = THREE.ClampToEdgeWrapping;
  tex.wrapT = THREE.RepeatWrapping;
  tex.minFilter = tex.magFilter = THREE.LinearFilter;
  tex.generateMipmaps = false;
  tex.needsUpdate = true;
  return tex;
}

// Sky colour by direction: the clear colour plus a low warm glow behind the ridges, right of centre.
const skyGlsl = /* glsl */ `
  uniform vec3 uBg;
  uniform float uDim;
  uniform float uOut;
  vec3 skyColor(vec3 d) {
    float az = dot(normalize(d.xz + vec2(1e-4)), normalize(vec2(0.35, -1.0)));
    float live = uDim * (1.0 - uOut);
    float haze = exp(-max(d.y, 0.0) * 7.0) * exp(-max(-d.y, 0.0) * 12.0);
    float band = exp(-abs(d.y) * 9.0) * pow(0.5 + 0.5 * az, 5.0);
    return uBg + vec3(0.013, 0.019, 0.032) * haze * live + vec3(0.16, 0.075, 0.035) * band * live * 0.24;
  }
`;

const terrainVert = /* glsl */ `
  ${heightGlsl}
  varying vec3 vWorld;
  varying vec3 vNormal;
  varying float vH;
  void main() {
    vec3 w = (modelMatrix * vec4(position, 1.0)).xyz;
    float h = terrainH(w.xz);
    float e = 0.6;
    float hx = terrainH(w.xz + vec2(e, 0.0));
    float hz = terrainH(w.xz + vec2(0.0, e));
    vNormal = normalize(vec3(h - hx, e, h - hz));
    vH = h;
    w.y = h - uSink;
    vWorld = w;
    gl_Position = projectionMatrix * viewMatrix * vec4(w, 1.0);
  }
`;

const terrainFrag = /* glsl */ `
  precision highp float;
  ${skyGlsl}
  uniform float uTime;
  uniform vec3 uCam;
  uniform float uFog;
  uniform vec3 uRim;
  uniform vec3 uBounce;
  uniform vec3 uMoon;
  uniform vec3 uBase;
  uniform vec3 uIso;
  varying vec3 vWorld;
  varying vec3 vNormal;
  varying float vH;
  void main() {
    vec3 N = normalize(vNormal);
    vec3 toCam = uCam - vWorld;
    float dist = length(toCam);
    vec3 V = toCam / dist;
    float up = clamp(N.y, 0.0, 1.0);
    // Valley floors sit in their own shadow.
    float ao = mix(0.45, 1.0, smoothstep(0.0, 6.0, vH));
    vec3 col = uBase * (0.6 + 0.4 * up) * ao;
    col += uBounce * pow(up, 1.6) * ao;
    // Moon high behind the camera, left: a cool key that gives the faces their form.
    vec3 M = normalize(vec3(-0.45, 0.55, 0.7));
    col += uMoon * pow(max(dot(N, M), 0.0), 1.3) * ao;
    // Low light far behind the ridges, a little right: it only catches crests and far faces.
    vec3 L = normalize(vec3(0.35, 0.1, -1.0));
    float ndl = max(dot(N, L), 0.0);
    float fres = pow(1.0 - max(dot(N, V), 0.0), 2.5);
    col += uRim * (pow(ndl, 3.0) * 0.7 + ndl * ndl * fres * 0.6) * (0.4 + 0.6 * smoothstep(0.0, 8.0, vH)) * smoothstep(30.0, 70.0, dist)
      * smoothstep(0.6, 0.92, dot(normalize(vWorld.xz - uCam.xz), normalize(vec2(0.35, -1.0))));
    col *= mix(0.4, 1.0, smoothstep(4.0, 32.0, dist));
    // Isolines: fract(height * k) crossing zero, creeping down the slopes. A hint, not a grid.
    float hk = vH * 1.1 - uTime * 0.04;
    float fw = fwidth(hk);
    float d = abs(fract(hk) - 0.5);
    float line = smoothstep(0.5 - 1.5 * fw, 0.5 - 0.3 * fw, d);
    line *= 1.0 - smoothstep(0.08, 0.3, fw);
    line *= 1.0 - smoothstep(10.0, 70.0, dist);
    // Gentle slopes only: on cliffs the lines bunch into a smear.
    line *= smoothstep(0.4, 2.5, vH) * smoothstep(0.06, 0.3, 1.0 - up) * (1.0 - smoothstep(0.5, 0.75, 1.0 - up));
    col += uIso * line * (0.35 + 0.65 * up);
    col *= uDim;
    float fog = 1.0 - exp(-dist * uFog);
    col = mix(col, skyColor(-V), fog);
    gl_FragColor = vec4(col, 1.0);
    #include <colorspace_fragment>
  }
`;

const skyVert = /* glsl */ `
  varying vec3 vDir;
  void main() {
    vDir = position;
    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    gl_Position = projectionMatrix * mv;
  }
`;

const skyFrag = /* glsl */ `
  precision highp float;
  ${skyGlsl}
  varying vec3 vDir;
  void main() {
    gl_FragColor = vec4(skyColor(normalize(vDir)), 1.0);
    #include <colorspace_fragment>
  }
`;

const lightsVert = /* glsl */ `
  ${heightGlsl}
  attribute float aSeed;
  attribute float aSize;
  uniform float uLift;
  uniform float uScale;
  varying float vA;
  varying float vSeed;
  void main() {
    vec3 p = position;
    float drifter = step(0.93, aSeed);
    p.x += drifter * sin(uTime * 0.07 + aSeed * 30.0) * 4.0;
    p.z += drifter * cos(uTime * 0.05 + aSeed * 12.0) * 3.0;
    p.y = terrainH(p.xz) - uSink + 0.3 + drifter * 2.5;
    p.y += sin(uTime * 0.7 + aSeed * 40.0) * 0.05;
    // Scroll: lights leave the ground and scatter upward.
    float rise = uLift * (3.0 + aSeed * 10.0);
    p.y += rise;
    p.x += uLift * sin(aSeed * 6.28 + uTime * 0.3) * 3.0;
    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    float near = smoothstep(4.0, 14.0, -mv.z);
    float flicker = 0.78 + 0.22 * sin(uTime * (2.0 + aSeed * 3.0) + aSeed * 50.0) * sin(uTime * 5.3 + aSeed * 9.0);
    vA = flicker * near * (1.0 - smoothstep(0.55, 0.9, uLift));
    vSeed = aSeed;
    gl_PointSize = clamp(aSize * uScale / -mv.z, 1.5, 90.0);
    gl_Position = projectionMatrix * mv;
  }
`;

const lightsFrag = /* glsl */ `
  precision highp float;
  uniform sampler2D uMap;
  uniform vec3 uWarm;
  uniform vec3 uCool;
  uniform float uDim;
  varying float vA;
  varying float vSeed;
  void main() {
    float a = texture2D(uMap, gl_PointCoord).a * vA * uDim;
    vec3 col = mix(uWarm, uCool, step(0.8, vSeed) * 0.7);
    gl_FragColor = vec4(col * a, a);
    #include <tonemapping_fragment>
    #include <colorspace_fragment>
  }
`;

const mistVert = /* glsl */ `
  ${heightGlsl}
  attribute float aSeed;
  attribute float aSize;
  uniform float uScale;
  uniform float uLift;
  varying float vA;
  void main() {
    vec3 p = position;
    p.x += sin(uTime * 0.05 + aSeed * 20.0) * 3.0;
    p.z += cos(uTime * 0.04 + aSeed * 7.0) * 2.0;
    p.y = terrainH(p.xz) - uSink + 1.0 + uLift * 6.0;
    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    vA = smoothstep(6.0, 18.0, -mv.z) * (0.75 + 0.25 * sin(uTime * 0.3 + aSeed * 30.0));
    // ponytail: some mobile GPUs clamp point size near 512; the mist then gets smaller, not harder.
    gl_PointSize = clamp(aSize * uScale / -mv.z, 2.0, 1024.0);
    gl_Position = projectionMatrix * mv;
  }
`;

const mistFrag = /* glsl */ `
  precision highp float;
  uniform vec3 uColor;
  uniform float uDim;
  varying float vA;
  void main() {
    float r = length(gl_PointCoord - 0.5) * 2.0;
    float a = pow(max(1.0 - r, 0.0), 2.2) * vA * uDim * 0.1;
    gl_FragColor = vec4(uColor, a);
    #include <colorspace_fragment>
  }
`;

const starsVert = /* glsl */ `
  attribute float aSeed;
  attribute float aSize;
  uniform float uTime;
  uniform float uDpr;
  varying float vA;
  void main() {
    float tw = step(0.7, aSeed) * pow(0.5 + 0.5 * sin(uTime * (1.0 + aSeed * 2.0) + aSeed * 90.0), 3.0);
    vA = mix(0.55, 1.0, tw) * (0.3 + 0.7 * aSeed);
    vec4 mv = modelViewMatrix * vec4(position, 1.0);
    gl_PointSize = aSize * uDpr;
    gl_Position = projectionMatrix * mv;
  }
`;

const starsFrag = /* glsl */ `
  precision highp float;
  uniform float uDim;
  varying float vA;
  void main() {
    float r = length(gl_PointCoord - 0.5) * 2.0;
    float a = pow(max(1.0 - r, 0.0), 1.5) * vA * uDim * 0.6;
    gl_FragColor = vec4(vec3(0.72, 0.8, 1.0), a);
    #include <colorspace_fragment>
  }
`;

function spriteTexture() {
  const size = 64;
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  const g = canvas.getContext("2d")!;
  const grad = g.createRadialGradient(size / 2, size / 2, 0, size / 2, size / 2, size / 2);
  grad.addColorStop(0, "rgba(255,255,255,1)");
  grad.addColorStop(0.18, "rgba(255,255,255,0.55)");
  grad.addColorStop(0.5, "rgba(255,255,255,0.12)");
  grad.addColorStop(1, "rgba(255,255,255,0)");
  g.fillStyle = grad;
  g.fillRect(0, 0, size, size);
  const tex = new THREE.CanvasTexture(canvas);
  tex.minFilter = THREE.LinearFilter;
  return tex;
}

const smooth = (a: number, b: number, x: number) => THREE.MathUtils.smoothstep(x, a, b);

export function createHero(): Chapter {
  const group = new THREE.Group();
  group.name = "hero";
  const disposables: Array<{ dispose(): void }> = [];
  let terrain: THREE.ShaderMaterial | null = null;
  let sky: THREE.ShaderMaterial | null = null;
  let lights: THREE.ShaderMaterial | null = null;
  let stars: THREE.ShaderMaterial | null = null;
  let mist: THREE.ShaderMaterial | null = null;
  let dome: THREE.Mesh | null = null;
  let enterMs = 0;
  let yaw = 0;
  let pitch = 0;
  const bg = new THREE.Color("#07080a");

  return {
    id: "hero",
    group,
    init(ctx) {
      const q = ctx.quality;
      const seg = q >= 2 ? 256 : 128;
      const tex = spriteTexture();
      const height = bakeHeightmap(512, 512, 260, 340);
      disposables.push(tex, height);

      // Terrain: camera near z = 8 looking down -z.
      const geo = new THREE.PlaneGeometry(260, 170, seg, seg);
      geo.rotateX(-Math.PI / 2);
      geo.translate(0, 0, -65);
      terrain = new THREE.ShaderMaterial({
        vertexShader: terrainVert,
        fragmentShader: terrainFrag,
        uniforms: {
          uHeight: { value: height },
          uTime: { value: 0 },
          uSink: { value: 0 },
          uDim: { value: 0 },
          uOut: { value: 0 },
          uFog: { value: 0.008 },
          uBg: { value: bg },
          uCam: { value: new THREE.Vector3() },
          uBase: { value: new THREE.Color("#0a0b0e") },
          uBounce: { value: new THREE.Color("#11161f") },
          uMoon: { value: new THREE.Color("#182030") },
          uRim: { value: new THREE.Color("#c2582a") },
          uIso: { value: new THREE.Color("#ff5b1f").multiplyScalar(0.12) },
        },
      });
      const ground = new THREE.Mesh(geo, terrain);
      ground.frustumCulled = false;
      disposables.push(geo, terrain);

      // Sky dome follows the camera; drawn first, no depth.
      const domeGeo = new THREE.SphereGeometry(300, 32, 16);
      sky = new THREE.ShaderMaterial({
        vertexShader: skyVert,
        fragmentShader: skyFrag,
        side: THREE.BackSide,
        depthWrite: false,
        depthTest: false,
        uniforms: { uBg: { value: bg }, uDim: { value: 0 }, uOut: { value: 0 } },
      });
      dome = new THREE.Mesh(domeGeo, sky);
      dome.renderOrder = -10;
      dome.frustumCulled = false;
      disposables.push(domeGeo, sky);

      // Valley lights: clustered like a far settlement along the corridor, a few strays.
      const nLights = q === 0 ? 40 : q === 1 ? 60 : 80;
      const lp = new Float32Array(nLights * 3);
      const ls = new Float32Array(nLights);
      const lz = new Float32Array(nLights);
      const towns = [-24, -42, -64];
      const tri = () => Math.random() + Math.random() - 1;
      for (let i = 0; i < nLights; i++) {
        const stray = i % 11 === 0;
        const cz = towns[i % towns.length];
        const z = stray ? -12 - Math.random() * 70 : cz + tri() * 6;
        const xv = 7 + 5 * Math.sin(z * 0.045 + 0.8);
        const x = xv + (stray ? (Math.random() - 0.5) * 16 : tri() * 4.5);
        lp[i * 3] = x;
        lp[i * 3 + 2] = z;
        ls[i] = Math.random();
        lz[i] = 0.35 + Math.random() * 0.5;
      }
      const lightsGeo = new THREE.BufferGeometry();
      lightsGeo.setAttribute("position", new THREE.BufferAttribute(lp, 3));
      lightsGeo.setAttribute("aSeed", new THREE.BufferAttribute(ls, 1));
      lightsGeo.setAttribute("aSize", new THREE.BufferAttribute(lz, 1));
      lights = new THREE.ShaderMaterial({
        vertexShader: lightsVert,
        fragmentShader: lightsFrag,
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
        uniforms: {
          uHeight: { value: height },
          uTime: { value: 0 },
          uSink: { value: 0 },
          uLift: { value: 0 },
          uScale: { value: 1000 },
          uDim: { value: 0 },
          uMap: { value: tex },
          uWarm: { value: new THREE.Color("#ffb067") },
          uCool: { value: new THREE.Color("#cfe3ff") },
        },
      });
      const lightPoints = new THREE.Points(lightsGeo, lights);
      lightPoints.frustumCulled = false;
      disposables.push(lightsGeo, lights);

      // Ground mist: a few big soft discs hugging the valley floor, so far ridges separate from near.
      const nMist = q === 0 ? 10 : 18;
      const mp = new Float32Array(nMist * 3);
      const ms = new Float32Array(nMist);
      const mz = new Float32Array(nMist);
      for (let i = 0; i < nMist; i++) {
        const z = -14 - (i / nMist) * 62 + (Math.random() - 0.5) * 4;
        mp[i * 3] = 7 + 5 * Math.sin(z * 0.045 + 0.8) + (Math.random() - 0.5) * 9;
        mp[i * 3 + 2] = z;
        ms[i] = Math.random();
        mz[i] = 7 + Math.random() * 5;
      }
      const mistGeo = new THREE.BufferGeometry();
      mistGeo.setAttribute("position", new THREE.BufferAttribute(mp, 3));
      mistGeo.setAttribute("aSeed", new THREE.BufferAttribute(ms, 1));
      mistGeo.setAttribute("aSize", new THREE.BufferAttribute(mz, 1));
      mist = new THREE.ShaderMaterial({
        vertexShader: mistVert,
        fragmentShader: mistFrag,
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
        uniforms: {
          uHeight: { value: height },
          uTime: { value: 0 },
          uSink: { value: 0 },
          uLift: { value: 0 },
          uScale: { value: 1000 },
          uDim: { value: 0 },
          uColor: { value: new THREE.Color("#3b4a66") },
        },
      });
      const mistPoints = new THREE.Points(mistGeo, mist);
      mistPoints.frustumCulled = false;
      disposables.push(mistGeo, mist);

      // Stars: a shell high up, occluded by the ridges.
      const nStars = q === 0 ? 300 : q === 1 ? 450 : 650;
      const sp = new Float32Array(nStars * 3);
      const ss = new Float32Array(nStars);
      const sz = new Float32Array(nStars);
      for (let i = 0; i < nStars; i++) {
        const a = Math.random() * Math.PI * 2;
        const el = 0.03 + Math.pow(Math.random(), 1.4) * 1.3;
        const r = 260;
        sp[i * 3] = Math.cos(a) * Math.cos(el) * r;
        sp[i * 3 + 1] = Math.sin(el) * r;
        sp[i * 3 + 2] = Math.sin(a) * Math.cos(el) * r;
        ss[i] = Math.random();
        sz[i] = 1.8 + Math.random() * 1.8;
      }
      const starsGeo = new THREE.BufferGeometry();
      starsGeo.setAttribute("position", new THREE.BufferAttribute(sp, 3));
      starsGeo.setAttribute("aSeed", new THREE.BufferAttribute(ss, 1));
      starsGeo.setAttribute("aSize", new THREE.BufferAttribute(sz, 1));
      stars = new THREE.ShaderMaterial({
        vertexShader: starsVert,
        fragmentShader: starsFrag,
        transparent: true,
        depthWrite: false,
        blending: THREE.AdditiveBlending,
        uniforms: { uTime: { value: 0 }, uDpr: { value: 1 }, uDim: { value: 0 } },
      });
      const starPoints = new THREE.Points(starsGeo, stars);
      starPoints.frustumCulled = false;
      disposables.push(starsGeo, stars);

      group.add(dome, ground, mistPoints, lightPoints, starPoints);
    },
    enter(ctx) {
      ctx.setBackground("#07080a", { near: 8, far: 60 });
      ctx.floor.set({ visible: false });
      ctx.setAccent("#ff5b1f");
      enterMs = performance.now();
    },
    exit() {},
    update(ctx, progress, dt, t) {
      if (!terrain || !sky || !lights || !stars || !mist || !dome) return;
      const p = progress;
      // Wall clock, not t: t is dt-capped and crawls on a slow machine.
      const fadeIn = smooth(0, 1.2, (performance.now() - enterMs) / 1000);
      const out = smooth(0.1, 0.9, p);
      const dim = fadeIn * (1 - smooth(0.45, 0.9, p));

      // Camera: low, horizon a little above centre; rises and tilts up as you scroll.
      const cam = ctx.camera;
      const rise = smooth(0.05, 0.95, p);
      cam.position.set(0, 3.4 + rise * 7, 8);
      const targetYaw = ctx.pointer.coarse ? Math.sin(t * 0.17) * 4 * DEG : -ctx.pointer.x * 6 * DEG;
      const targetPitch = ctx.pointer.coarse ? Math.sin(t * 0.11) * 1.5 * DEG : -ctx.pointer.y * 3 * DEG;
      const k = 1 - Math.exp(-dt * 3);
      yaw += (targetYaw - yaw) * k;
      pitch += (targetPitch - pitch) * k;
      cam.rotation.order = "YXZ";
      cam.rotation.set(-3 * DEG + rise * 11 * DEG + pitch, (ctx.viewport.mobile ? -5 * DEG : 0) + yaw, 0);
      dome.position.copy(cam.position);

      const fog = ctx.scene.fog as THREE.Fog | null;
      bg.copy(fog ? fog.color : (ctx.scene.background as THREE.Color));

      const tu = terrain.uniforms;
      tu.uTime.value = t;
      tu.uSink.value = out * 5;
      tu.uDim.value = dim;
      tu.uOut.value = out;
      tu.uFog.value = 0.008 + out * 0.07;
      (tu.uCam.value as THREE.Vector3).copy(cam.position);
      sky.uniforms.uDim.value = dim;
      sky.uniforms.uOut.value = out;

      const lu = lights.uniforms;
      lu.uTime.value = t;
      lu.uSink.value = out * 5;
      lu.uLift.value = smooth(0.12, 0.85, p);
      const px = ctx.viewport.height * ctx.viewport.dpr;
      lu.uScale.value = (px * 0.5) / Math.tan(cam.fov * 0.5 * DEG);
      lu.uDim.value = dim;

      const mu = mist.uniforms;
      mu.uTime.value = t;
      mu.uSink.value = out * 5;
      mu.uLift.value = out;
      mu.uScale.value = lu.uScale.value;
      mu.uDim.value = dim * (1 - out);

      const su = stars.uniforms;
      su.uTime.value = t;
      su.uDpr.value = ctx.viewport.dpr;
      su.uDim.value = dim;
    },
    dispose() {
      for (const d of disposables) d.dispose();
      disposables.length = 0;
    },
  };
}
