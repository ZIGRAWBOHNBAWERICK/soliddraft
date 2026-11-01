/**
 * A stand-in chapter so the page and the lab run before a real scene exists.
 * Replace the whole file of a chapter with the real thing; keep the exported factory name.
 */
import * as THREE from "three";

import type { Chapter, ChapterContext, ChapterId } from "../types";

export function createPlaceholder(id: ChapterId, color: string): Chapter {
  const group = new THREE.Group();
  group.name = id;
  let mesh: THREE.Mesh | null = null;
  return {
    id,
    group,
    init() {
      const geometry = new THREE.IcosahedronGeometry(0.9, 1);
      const material = new THREE.MeshStandardMaterial({
        color: "#15181c",
        roughness: 0.4,
        metalness: 0.7,
        wireframe: false,
      });
      mesh = new THREE.Mesh(geometry, material);
      mesh.position.y = 1.1;
      const edges = new THREE.LineSegments(
        new THREE.EdgesGeometry(geometry, 1),
        new THREE.LineBasicMaterial({ color, transparent: true, opacity: 0.9 }),
      );
      mesh.add(edges);
      const light = new THREE.PointLight(color, 20, 12, 2);
      light.position.set(2, 3, 2);
      group.add(mesh, light, new THREE.AmbientLight("#ffffff", 0.15));
    },
    enter(ctx: ChapterContext) {
      ctx.setBackground("#07080a", { near: 6, far: 40 });
      ctx.floor.set({ color, intensity: 0.9, visible: true });
      ctx.setAccent(color);
    },
    exit() {},
    update(ctx, progress, _dt, t) {
      if (!mesh) return;
      mesh.rotation.y = t * 0.4 + progress * Math.PI * 2;
      mesh.rotation.x = Math.sin(t * 0.3) * 0.2;
      const fade = Math.min(1, progress / 0.06, (1 - progress) / 0.06);
      mesh.scale.setScalar(THREE.MathUtils.lerp(0.001, 1, THREE.MathUtils.clamp(fade, 0, 1)));
      const yaw = ctx.pointer.x * 0.25;
      const pitch = ctx.pointer.y * 0.12;
      ctx.camera.position.set(Math.sin(yaw) * 6, 1.6 + pitch * 2, Math.cos(yaw) * 6);
      ctx.camera.lookAt(0, 1.0, 0);
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
