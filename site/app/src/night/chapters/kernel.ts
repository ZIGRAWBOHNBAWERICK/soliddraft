/**
 * Kernel chapter: the Catallax geometry kernel under SolidDraft. The rounded boss (a real
 * SolidDraft part with two fillets) in dark metal, its B-rep edges traced in oxide, dissolves
 * into a particle cloud that re-forms as SolidDraft's lattice block in emerald. Proven solid
 * (0 to 0.42), certified field (0.6 to 1).
 */
import * as THREE from "three";
import { MeshSurfaceSampler } from "three/examples/jsm/math/MeshSurfaceSampler.js";

import { fitPart } from "../assets";
import type { Chapter, ChapterContext } from "../types";

const OXIDE = "#ff5b1f";
const EMERALD = "#35c27c";
const METAL = "#1a1d22";
/** Progress where the housing starts to dissolve (whoosh) and where the lattice is complete (chime). */
const DISSOLVE = 0.42;
const LATTICE_DONE = 0.7;
/** Yaw of the lattice block once it has formed. */
const LATTICE_YAW = Math.PI - 0.6;
const SUBJECT = new THREE.Vector3(0.3, 1.1, 0);
const ORBIT = 4.5;

const smooth = (a: number, b: number, x: number) => THREE.MathUtils.smoothstep(x, a, b);

const pointsVert = /* glsl */ `
  uniform float uMix;
  uniform float uTime;
  uniform float uSize;
  uniform float uScale;
  uniform float uFromYaw;
  uniform float uToYaw;
  attribute vec3 aTo;
  attribute vec3 aSeed;
  varying float vMix;
  varying float vSeed;

  vec3 rotY(vec3 v, float a) {
    float c = cos(a);
    float s = sin(a);
    return vec3(c * v.x + s * v.z, v.y, -s * v.x + c * v.z);
  }

  // Curl of a sine potential: divergence free, so the cloud swirls instead of streaming.
  vec3 curl(vec3 p, float t) {
    float a = 2.5 * p.x + 1.9 * p.y + t;
    float b = 1.7 * p.z + 2.3 * p.x + t * 0.8;
    float c = 2.1 * p.y + 1.3 * p.z + t * 1.2;
    return vec3(1.9 * cos(a) - 1.7 * cos(b), 1.3 * cos(c) - 2.5 * cos(a), 2.3 * cos(b) - 2.1 * cos(c));
  }

  void main() {
    // Each point starts a little later than the last so the surface peels instead of jumping.
    float d = clamp((uMix - aSeed.x * 0.3) / 0.7, 0.0, 1.0);
    float e = d * d * (3.0 - 2.0 * d);
    float swell = sin(e * 3.14159);
    // Housing and lattice turn independently; each end of the path lives in its own frame.
    vec3 from = rotY(position, uFromYaw);
    vec3 p = mix(from, rotY(aTo, uToYaw), e);
    vec3 outward = normalize(from + vec3(0.0, 0.001, 0.0));
    p += outward * swell * (0.35 + aSeed.y * 0.9);
    p += curl(from * 2.0 + aSeed * 3.0, uTime * 0.6) * swell * 0.22;
    // Settled points still breathe.
    p += vec3(
      sin(uTime * 1.7 + aSeed.x * 40.0),
      cos(uTime * 1.3 + aSeed.y * 40.0),
      sin(uTime * 1.1 + aSeed.z * 40.0)
    ) * 0.004;
    vec4 mv = modelViewMatrix * vec4(p, 1.0);
    gl_Position = projectionMatrix * mv;
    gl_PointSize = uSize * (0.6 + aSeed.y * 0.8) * (1.0 + swell * 1.2) * uScale / -mv.z;
    vMix = e;
    vSeed = aSeed.z;
  }
`;

const pointsFrag = /* glsl */ `
  precision highp float;
  uniform vec3 uColorA;
  uniform vec3 uColorB;
  uniform float uAlpha;
  uniform float uSparkle;
  uniform float uTime;
  varying float vMix;
  varying float vSeed;
  void main() {
    float r = length(gl_PointCoord - 0.5);
    if (r > 0.5) discard;
    float soft = smoothstep(0.5, 0.05, r);
    soft *= soft;
    vec3 col = mix(uColorA, uColorB, smoothstep(0.45, 0.85, vMix));
    float tw = 0.5 + 0.5 * sin(uTime * 3.0 + vSeed * 60.0);
    // At the end only a few points stay, twinkling.
    float keep = mix(1.0, step(0.965, vSeed) * (0.2 + 0.8 * tw), uSparkle);
    gl_FragColor = vec4(col * (0.7 + 0.3 * soft), soft * uAlpha * keep);
    #include <tonemapping_fragment>
    #include <colorspace_fragment>
  }
`;

export function createKernel(): Chapter {
  const group = new THREE.Group();
  group.name = "kernel";
  // tilt leans the turntable toward the camera so the bore and holes read; subject spins on it.
  const tilt = new THREE.Group();
  const subject = new THREE.Group();
  tilt.position.copy(SUBJECT);
  tilt.add(subject);
  group.add(tilt);

  let housing: THREE.Mesh<THREE.BufferGeometry, THREE.MeshPhysicalMaterial> | null = null;
  let edges: THREE.LineSegments<THREE.BufferGeometry, THREE.LineBasicMaterial> | null = null;
  let lattice: THREE.Mesh<THREE.BufferGeometry, THREE.MeshPhysicalMaterial> | null = null;
  let points: THREE.Points<THREE.BufferGeometry, THREE.ShaderMaterial> | null = null;
  let latticeEdges: THREE.LineSegments<THREE.BufferGeometry, THREE.LineBasicMaterial> | null = null;
  let pointCount = 0;

  const key = new THREE.DirectionalLight("#ffffff", 2.4);
  key.position.set(3, 5, 3);
  const rim = new THREE.PointLight(OXIDE, 60, 16, 2);
  rim.position.set(-2.6, 2.4, -2.2);
  const fill = new THREE.PointLight("#9fb4ff", 14, 12, 2);
  fill.position.set(2.5, 0.6, 3);
  const bore = new THREE.PointLight(OXIDE, 0, 3.5, 2);
  subject.add(bore);
  // Lattice phase: a second light at the far end of the pocket, an emerald rim and a white key.
  const pocket = new THREE.PointLight(EMERALD, 0, 2.5, 2);
  pocket.position.set(0.5, 0.05, 0);
  const latticeRim = new THREE.PointLight(EMERALD, 0, 18, 2);
  latticeRim.position.set(-2.0, 2.6, -1.8);
  const latticeFill = new THREE.PointLight("#ffffff", 0, 14, 2);
  latticeFill.position.set(1.2, 2.6, 2.4);
  const latticeKey = new THREE.DirectionalLight("#ffffff", 0);
  latticeKey.position.set(2.5, 4.5, 1.2);
  group.add(key, rim, fill, latticeRim, latticeKey, latticeFill);

  const oxide = new THREE.Color(OXIDE);
  const emerald = new THREE.Color(EMERALD);
  const look = new THREE.Vector3();
  let prev = -1;
  let fresh = true;
  let emeraldOn = false;

  const applyAccent = (ctx: ChapterContext, hi: boolean) => {
    emeraldOn = hi;
    const c = hi ? EMERALD : OXIDE;
    ctx.setAccent(c);
    ctx.floor.set({ color: c, intensity: 0.9, visible: true });
  };

  return {
    id: "kernel",
    group,

    async init(ctx) {
      const [h, l] = await Promise.all([
        ctx.assets.gltf("/assets/3d/boss.glb"),
        ctx.assets.gltf("/assets/3d/lattice.glb"),
      ]);
      const housingGeo = fitPart(h, 1.6).geometry;
      const latticeGeo = fitPart(l, 1.6).geometry;

      housing = new THREE.Mesh(
        housingGeo,
        new THREE.MeshPhysicalMaterial({
          color: METAL,
          metalness: 0.85,
          roughness: 0.35,
          clearcoat: 0.3,
          clearcoatRoughness: 0.25,
          transparent: true,
          opacity: 0,
        }),
      );
      edges = new THREE.LineSegments(
        new THREE.EdgesGeometry(housingGeo, 25),
        new THREE.LineBasicMaterial({
          color: oxide.clone().multiplyScalar(3.0),
          transparent: true,
          opacity: 0,
          blending: THREE.AdditiveBlending,
          depthWrite: false,
        }),
      );
      edges.renderOrder = 1;
      lattice = new THREE.Mesh(
        latticeGeo,
        new THREE.MeshPhysicalMaterial({
          color: METAL,
          metalness: 0.85,
          roughness: 0.35,
          clearcoat: 0.3,
          clearcoatRoughness: 0.25,
          emissive: EMERALD,
          emissiveIntensity: 0,
          envMapIntensity: 2.2,
          transparent: true,
          opacity: 0,
        }),
      );
      lattice.visible = false;
      lattice.add(pocket);
      latticeEdges = new THREE.LineSegments(
        new THREE.EdgesGeometry(latticeGeo, 25),
        new THREE.LineBasicMaterial({
          color: emerald.clone().multiplyScalar(3.5),
          transparent: true,
          opacity: 0,
          blending: THREE.AdditiveBlending,
          depthWrite: false,
        }),
      );
      latticeEdges.renderOrder = 1;
      latticeEdges.visible = false;

      // Particles: one per sampled housing surface point, each with a sampled lattice target.
      pointCount = [12000, 20000, 30000][ctx.quality];
      const from = new Float32Array(pointCount * 3);
      const to = new Float32Array(pointCount * 3);
      const seed = new Float32Array(pointCount * 3);
      const fromSampler = new MeshSurfaceSampler(housing).build();
      const toSampler = new MeshSurfaceSampler(lattice).build();
      const v = new THREE.Vector3();
      for (let i = 0; i < pointCount; i++) {
        const o = i * 3;
        fromSampler.sample(v);
        from[o] = v.x;
        from[o + 1] = v.y;
        from[o + 2] = v.z;
        toSampler.sample(v);
        to[o] = v.x;
        to[o + 1] = v.y;
        to[o + 2] = v.z;
        seed[o] = Math.random();
        seed[o + 1] = Math.random();
        seed[o + 2] = Math.random();
      }
      const pointsGeo = new THREE.BufferGeometry();
      pointsGeo.setAttribute("position", new THREE.BufferAttribute(from, 3));
      pointsGeo.setAttribute("aTo", new THREE.BufferAttribute(to, 3));
      pointsGeo.setAttribute("aSeed", new THREE.BufferAttribute(seed, 3));
      points = new THREE.Points(
        pointsGeo,
        new THREE.ShaderMaterial({
          vertexShader: pointsVert,
          fragmentShader: pointsFrag,
          transparent: true,
          depthWrite: false,
          blending: THREE.AdditiveBlending,
          uniforms: {
            uMix: { value: 0 },
            uTime: { value: 0 },
            uSize: { value: 0.036 },
            uScale: { value: 450 },
            uAlpha: { value: 0 },
            uSparkle: { value: 0 },
            uFromYaw: { value: 0 },
            uToYaw: { value: 0 },
            uColorA: { value: oxide },
            uColorB: { value: emerald },
          },
        }),
      );
      points.frustumCulled = false;
      points.renderOrder = 2;
      points.visible = false;

      subject.add(housing, edges, lattice, latticeEdges, points);
    },

    enter(ctx) {
      ctx.setBackground("#07080a", { near: 6, far: 30 });
      applyAccent(ctx, false);
      fresh = true;
    },

    exit() {},

    update(ctx, p, _dt, t) {
      if (fresh) {
        prev = p;
        fresh = false;
      }
      // Cues fire on forward crossings only; going back below re-arms them.
      if (prev < DISSOLVE && p >= DISSOLVE) ctx.cue("whoosh");
      if (prev < LATTICE_DONE && p >= LATTICE_DONE) ctx.cue("chime");
      prev = p;
      if (p >= 0.5 !== emeraldOn) applyAccent(ctx, p >= 0.5);

      const fadeIn = smooth(0, 0.1, p);
      const fadeOut = 1 - smooth(0.94, 1, p);
      const housingVis = fadeIn * (1 - smooth(DISSOLVE, 0.56, p));
      const edgeVis = smooth(0.06, 0.16, p) * (1 - smooth(0.4, 0.47, p));
      const mix = smooth(DISSOLVE, 0.62, p);
      const latticeVis = smooth(0.6, LATTICE_DONE, p) * fadeOut;
      const pointAlpha = smooth(0.4, 0.46, p) * fadeOut * (1 - 0.85 * latticeVis);
      const sparkle = smooth(0.8, 0.9, p);
      const colourMix = smooth(0.45, 0.65, p);
      const pulse = 0.7 + 0.3 * Math.sin(t * 2.2);

      tilt.rotation.x = 0.55;
      tilt.rotation.z = Math.sin(t * 0.27) * 0.06;
      // The housing spins on; the lattice holds a pose that looks into the pocket and sways.
      const spin = t * 0.25 + p * 2.5;
      const pose = LATTICE_YAW + Math.sin(t * 0.3) * 0.25;
      const slow = 0.5 + 0.5 * Math.sin(t * 1.1);

      if (housing) {
        housing.visible = housingVis > 0.001;
        housing.material.opacity = housingVis;
        housing.material.transparent = housingVis < 0.999;
        housing.scale.setScalar(0.6 + 0.4 * fadeIn);
        housing.rotation.y = spin;
      }
      if (edges) {
        edges.visible = edgeVis > 0.001;
        edges.material.opacity = edgeVis * (0.55 + 0.45 * pulse);
        edges.scale.copy(housing!.scale);
        edges.rotation.y = spin;
      }
      bore.intensity = 10 * edgeVis * pulse + 7 * latticeVis * (0.6 + 0.4 * slow);
      pocket.intensity = 6 * latticeVis * (1 - 0.4 * slow);
      bore.color.lerpColors(oxide, emerald, colourMix);
      if (points) {
        const u = points.material.uniforms;
        points.visible = pointAlpha > 0.001;
        u.uMix.value = mix;
        u.uTime.value = t;
        u.uAlpha.value = pointAlpha * 0.6;
        u.uSparkle.value = sparkle;
        u.uScale.value = ctx.viewport.dpr * ctx.viewport.height * 0.5;
        u.uFromYaw.value = spin;
        u.uToYaw.value = pose;
      }
      if (lattice) {
        lattice.visible = latticeVis > 0.001;
        lattice.material.opacity = latticeVis;
        lattice.material.emissiveIntensity = latticeVis * (0.03 + 0.015 * Math.sin(t * 1.8));
        lattice.material.transparent = latticeVis < 0.999;
        lattice.rotation.y = pose;
      }
      if (latticeEdges) {
        latticeEdges.visible = latticeVis > 0.001;
        latticeEdges.rotation.y = pose;
        latticeEdges.material.opacity = latticeVis * (0.35 + 0.25 * (0.5 + 0.5 * Math.sin(t * 1.4)));
      }

      const lit = Math.min(1, fadeIn * 4) * fadeOut;
      key.intensity = 2.4 * lit;
      fill.intensity = 14 * lit;
      rim.intensity = 60 * lit;
      rim.color.lerpColors(oxide, emerald, colourMix);
      latticeRim.intensity = 150 * latticeVis * lit;
      latticeFill.intensity = 120 * latticeVis * lit;
      latticeKey.intensity = 3 * latticeVis * lit;

      // Camera: slow orbit, pointer parallax on desktop, auto orbit on touch.
      const yaw = ctx.pointer.coarse ? t * 0.12 : t * 0.03 + ctx.pointer.x * 0.28;
      const pitch = 0.2 - (ctx.pointer.coarse ? 0 : ctx.pointer.y * 0.12);
      const orbit = ctx.viewport.mobile ? 6.2 : ORBIT;
      const side = ctx.viewport.mobile ? 0.05 : 0.45;
      const cp = Math.cos(pitch) * orbit;
      ctx.camera.position.set(
        SUBJECT.x + Math.sin(yaw) * cp,
        SUBJECT.y + Math.sin(pitch) * orbit,
        SUBJECT.z + Math.cos(yaw) * cp,
      );
      // Aim a little to the camera's left so the subject sits right of centre, clear of the caption.
      look.set(SUBJECT.x - Math.cos(yaw) * side, SUBJECT.y - 0.1, SUBJECT.z + Math.sin(yaw) * side);
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
    },
  };
}
