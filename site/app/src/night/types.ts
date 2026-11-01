/**
 * The contract every chapter (one 3D scene of the scroll) implements.
 * The stage owns the renderer, the camera, the dot floor and the frame loop.
 * A chapter owns one THREE.Group, its lights, its materials and its camera moves.
 */
import type * as THREE from "three";
import type { GLTF } from "three/examples/jsm/loaders/GLTFLoader.js";

export type ChapterId = "hero" | "kernel" | "cad" | "draw" | "screen" | "light" | "smoke";

export interface Pointer {
  /** -1..1, left to right, smoothed. 0 when no pointer (touch, headless). */
  x: number;
  /** -1..1, top to bottom, smoothed. */
  y: number;
  /** Smoothed velocity per second in pointer units. */
  vx: number;
  vy: number;
  /** True while a mouse button or finger is down. */
  down: boolean;
  /** True on touch devices; chapters should auto-orbit instead of following the pointer. */
  coarse: boolean;
}

export interface Viewport {
  width: number;
  height: number;
  dpr: number;
  aspect: number;
  /** Narrow screen: lower counts, bigger type, safe framing. */
  mobile: boolean;
}

export interface FloorControl {
  /** Tint, brightness 0..1.5 and visibility of the shared dot grid. Values ease in over ~0.6 s. */
  set(opts: { color?: THREE.ColorRepresentation; intensity?: number; visible?: boolean }): void;
  /** Expanding ring from (x, z) on the floor. strength 0..1. */
  ripple(x: number, z: number, strength?: number): void;
}

export interface Assets {
  /** GLB from /assets/3d, cached: real SolidDraft exports (bracket, collar, boss, tray, plate, fillet, lattice, gyroid) in millimetres, Y up. */
  gltf(url: string): Promise<GLTF>;
  /** Image texture, cached, colorSpace set to sRGB. */
  texture(url: string): Promise<THREE.Texture>;
}

export interface ChapterContext {
  renderer: THREE.WebGLRenderer;
  scene: THREE.Scene;
  camera: THREE.PerspectiveCamera;
  /** PMREM room environment. Assign to MeshStandardMaterial.envMap or use scene.environment (already set). */
  env: THREE.Texture;
  pointer: Pointer;
  viewport: Viewport;
  floor: FloorControl;
  assets: Assets;
  /** 0 low (phones), 1 mid, 2 high. Scale particle counts and segment counts with it. */
  quality: 0 | 1 | 2;
  /** Camera shake impulse, 0..1. Decays by itself. */
  shake(strength: number): void;
  /** Clear colour and fog. Eases over ~0.8 s. Call in enter(). */
  setBackground(color: THREE.ColorRepresentation, fog?: { near: number; far: number }): void;
  /** Tint the page UI (captions, rail marker, buttons). Call in enter() or on sub-beats. */
  setAccent(color: string): void;
  /** Play a short sound cue if the visitor turned sound on. */
  cue(name: "tick" | "thud" | "chime" | "whoosh"): void;
  /**
   * Render at a fraction of the display resolution while this chapter is active (0.3 to 1).
   * For full-screen shader work that is soft anyway. Reset to 1 when the chapter exits.
   */
  setRenderScale(scale: number): void;
}

export interface Chapter {
  readonly id: ChapterId;
  /** Added to ctx.scene by the stage. Keep everything of yours under it. */
  readonly group: THREE.Group;
  /** Build once, lazily, before the first enter. Load GLBs and textures here. */
  init(ctx: ChapterContext): Promise<void> | void;
  /** Become the active chapter. Set background, floor tint, accent. Make the group visible. */
  enter(ctx: ChapterContext): void;
  /** Stop being active. Hide the group. Never dispose here. */
  exit(ctx: ChapterContext): void;
  /**
   * Every frame while active. progress is 0..1 across the chapter's scroll span and is
   * already smoothed by the stage. dt is seconds since the last frame (capped), t is seconds
   * since boot. Set ctx.camera.position and orientation here; the stage applies shake after.
   * Fade your content in over the first ~0.06 of progress and out over the last ~0.06 so
   * chapter changes never cut.
   */
  update(ctx: ChapterContext, progress: number, dt: number, t: number): void;
  resize?(ctx: ChapterContext): void;
  dispose(): void;
}
