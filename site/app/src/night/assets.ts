import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import type { GLTF } from "three/examples/jsm/loaders/GLTFLoader.js";

import type { Assets } from "./types";

/** A site-relative URL ("/assets/...") with the deploy base in front. */
const withBase = (url: string) => (url.startsWith("/") ? `${import.meta.env.BASE_URL}${url.slice(1)}` : url);

/** Cached loaders shared by every chapter. Chapters name files as "/assets/..."; the base is added here. */
export function createAssets(): Assets {
  const gltfLoader = new GLTFLoader();
  const textureLoader = new THREE.TextureLoader();
  const gltfs = new Map<string, Promise<GLTF>>();
  const textures = new Map<string, Promise<THREE.Texture>>();
  return {
    gltf(url) {
      let p = gltfs.get(url);
      if (!p) {
        p = gltfLoader.loadAsync(withBase(url));
        gltfs.set(url, p);
      }
      return p;
    },
    texture(url) {
      let p = textures.get(url);
      if (!p) {
        p = textureLoader.loadAsync(withBase(url)).then((tex) => {
          tex.colorSpace = THREE.SRGBColorSpace;
          tex.anisotropy = 4;
          return tex;
        });
        textures.set(url, p);
      }
      return p;
    },
  };
}

/**
 * The GLBs are exported in millimetres (a bracket is ~120 units long). This returns the first
 * mesh of a GLB, recentred on its bounding box and scaled so its longest side is `size`.
 * Blender exported smooth normals everywhere and its booleans left sliver triangles, which
 * shade flat plates like pyramids, so the normals are recomputed with a 30 degree crease and
 * area weighting: flat faces stay flat, cylinders stay smooth, slivers count for nothing.
 */
export function fitPart(gltf: GLTF, size = 1): THREE.Mesh {
  let mesh: THREE.Mesh | null = null;
  gltf.scene.traverse((o) => {
    if (!mesh && (o as THREE.Mesh).isMesh) mesh = o as THREE.Mesh;
  });
  if (!mesh) throw new Error("GLB has no mesh");
  const m = (mesh as THREE.Mesh).clone();
  m.geometry = creasedNormals(m.geometry, Math.PI / 6);
  m.geometry.computeBoundingBox();
  const box = m.geometry.boundingBox!;
  const centre = box.getCenter(new THREE.Vector3());
  const span = box.getSize(new THREE.Vector3());
  const s = size / Math.max(span.x, span.y, span.z);
  m.geometry.translate(-centre.x, -centre.y, -centre.z);
  m.geometry.scale(s, s, s);
  m.geometry.computeBoundingBox();
  m.geometry.computeBoundingSphere();
  m.position.set(0, 0, 0);
  m.rotation.set(0, 0, 0);
  m.scale.set(1, 1, 1);
  return m;
}

/** Non-indexed copy of `src` with area-weighted vertex normals, smoothed only across faces within `creaseAngle`. */
export function creasedNormals(src: THREE.BufferGeometry, creaseAngle: number): THREE.BufferGeometry {
  const geo = src.index ? src.toNonIndexed() : src.clone();
  const pos = geo.attributes.position as THREE.BufferAttribute;
  const corners = pos.count;
  const faces = corners / 3;
  const faceN = new Float32Array(faces * 3); // unnormalised cross product: direction and twice the area
  const faceArea = new Float32Array(faces);
  const a = new THREE.Vector3();
  const b = new THREE.Vector3();
  const c = new THREE.Vector3();
  for (let f = 0; f < faces; f++) {
    a.fromBufferAttribute(pos, f * 3);
    b.fromBufferAttribute(pos, f * 3 + 1).sub(a);
    c.fromBufferAttribute(pos, f * 3 + 2).sub(a);
    b.cross(c);
    faceN[f * 3] = b.x;
    faceN[f * 3 + 1] = b.y;
    faceN[f * 3 + 2] = b.z;
    faceArea[f] = b.length();
  }
  const buckets = new Map<string, number[]>();
  for (let i = 0; i < corners; i++) {
    const key = `${Math.round(pos.getX(i) * 1e4)},${Math.round(pos.getY(i) * 1e4)},${Math.round(pos.getZ(i) * 1e4)}`;
    let list = buckets.get(key);
    if (!list) buckets.set(key, (list = []));
    list.push(i);
  }
  const cosCrease = Math.cos(creaseAngle);
  const out = new Float32Array(corners * 3);
  const n = new THREE.Vector3();
  const nf = new THREE.Vector3();
  const nj = new THREE.Vector3();
  // Slivers from the Blender booleans: anything under 1e-8 of the part's squared extent.
  geo.computeBoundingBox();
  const extent = geo.boundingBox!.getSize(a).length();
  const tiny = extent * extent * 1e-8;
  for (let i = 0; i < corners; i++) {
    const f = Math.floor(i / 3);
    const degenerate = faceArea[f] < tiny;
    nf.set(faceN[f * 3], faceN[f * 3 + 1], faceN[f * 3 + 2]).normalize();
    n.set(0, 0, 0);
    const key = `${Math.round(pos.getX(i) * 1e4)},${Math.round(pos.getY(i) * 1e4)},${Math.round(pos.getZ(i) * 1e4)}`;
    for (const j of buckets.get(key)!) {
      const fj = Math.floor(j / 3);
      if (faceArea[fj] < tiny) continue;
      nj.set(faceN[fj * 3], faceN[fj * 3 + 1], faceN[fj * 3 + 2]);
      if (degenerate || nj.dot(nf) / faceArea[fj] >= cosCrease) n.add(nj);
    }
    if (n.lengthSq() < 1e-20) n.copy(nf);
    n.normalize();
    out[i * 3] = n.x;
    out[i * 3 + 1] = n.y;
    out[i * 3 + 2] = n.z;
  }
  geo.setAttribute("normal", new THREE.BufferAttribute(out, 3));
  return geo;
}
