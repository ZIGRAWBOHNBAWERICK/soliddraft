/**
 * One renderer, one camera, one frame loop. The stage decides which chapter is active from the
 * progress the scroll system hands it, calls that chapter's update, applies camera shake and
 * renders. Chapters are initialised lazily and preloaded in the background after boot.
 */
import * as THREE from "three";
import { RoomEnvironment } from "three/examples/jsm/environments/RoomEnvironment.js";

import { createAssets } from "./assets";
import { Floor } from "./floor";
import type { Chapter, ChapterContext, ChapterId, Pointer, Viewport } from "./types";

export interface StageOptions {
  canvas: HTMLCanvasElement;
  chapters: Chapter[];
  quality: 0 | 1 | 2;
  onAccent?: (color: string) => void;
  cue?: ChapterContext["cue"];
}

export class Stage {
  readonly renderer: THREE.WebGLRenderer;
  readonly scene = new THREE.Scene();
  readonly camera: THREE.PerspectiveCamera;
  readonly ctx: ChapterContext;
  readonly chapters: Map<ChapterId, Chapter>;

  private readonly floor = new Floor();
  private last = 0;
  private readonly ready = new Map<ChapterId, Promise<void>>();
  private active: Chapter | null = null;
  private activeReady = false;
  private target = 0;
  private progress = 0;
  private shakeAmp = 0;
  private shakeSeed = 0;
  private readonly bgTarget = new THREE.Color("#07080a");
  private fogTarget: { near: number; far: number } | null = null;
  private readonly rawPointer = { x: 0, y: 0 };
  private readonly pointer: Pointer;
  private readonly viewport: Viewport;
  private raf = 0;
  private running = false;
  private time = 0;
  private readonly cleanups: Array<() => void> = [];
  private readonly shakeOffset = new THREE.Vector3();
  private renderScale = 1;

  constructor(private readonly options: StageOptions) {
    const { canvas, quality } = options;
    this.renderer = new THREE.WebGLRenderer({
      canvas,
      antialias: quality > 0,
      alpha: false,
      powerPreference: "high-performance",
      stencil: false,
    });
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.toneMapping = THREE.ACESFilmicToneMapping;
    this.renderer.toneMappingExposure = 1.05;
    this.renderer.localClippingEnabled = true;
    this.renderer.setClearColor(this.bgTarget, 1);

    this.camera = new THREE.PerspectiveCamera(42, 1, 0.05, 400);
    this.camera.position.set(0, 1.6, 6);

    const pmrem = new THREE.PMREMGenerator(this.renderer);
    const env = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
    pmrem.dispose();
    this.scene.environment = env;
    this.scene.environmentIntensity = 0.6;
    this.scene.background = this.bgTarget.clone();
    this.scene.add(this.floor.mesh);

    const coarse = typeof matchMedia === "function" && matchMedia("(pointer: coarse)").matches;
    this.pointer = { x: 0, y: 0, vx: 0, vy: 0, down: false, coarse };
    this.viewport = { width: 1, height: 1, dpr: 1, aspect: 1, mobile: false };
    this.chapters = new Map(options.chapters.map((c) => [c.id, c]));

    this.ctx = {
      renderer: this.renderer,
      scene: this.scene,
      camera: this.camera,
      env,
      pointer: this.pointer,
      viewport: this.viewport,
      floor: this.floor.control,
      assets: createAssets(),
      quality,
      shake: (strength) => {
        this.shakeAmp = Math.min(1.5, this.shakeAmp + strength);
        this.shakeSeed = Math.random() * 100;
      },
      setBackground: (color, fog) => {
        this.bgTarget.set(color);
        this.fogTarget = fog ?? null;
        if (fog && !this.scene.fog) this.scene.fog = new THREE.Fog(this.bgTarget.clone(), fog.near, fog.far);
      },
      setAccent: (color) => options.onAccent?.(color),
      cue: (name) => options.cue?.(name),
      setRenderScale: (scale) => {
        const next = THREE.MathUtils.clamp(scale, 0.3, 1);
        if (next === this.renderScale) return;
        this.renderScale = next;
        this.resize();
      },
    };

    for (const chapter of options.chapters) {
      chapter.group.visible = false;
      this.scene.add(chapter.group);
    }

    this.bindPointer(canvas);
    this.resize();
    const onResize = () => this.resize();
    window.addEventListener("resize", onResize);
    this.cleanups.push(() => window.removeEventListener("resize", onResize));
  }

  /** Make `id` the active chapter at `progress` (0..1). Called by the scroll system every frame it changes. */
  setActive(id: ChapterId, progress: number) {
    const chapter = this.chapters.get(id);
    if (!chapter) return;
    this.target = THREE.MathUtils.clamp(progress, 0, 1);
    if (this.active === chapter) return;
    if (this.active) {
      this.active.exit(this.ctx);
      this.active.group.visible = false;
      this.ctx.setRenderScale(1);
    }
    this.active = chapter;
    this.activeReady = false;
    // Jump the smoothed progress so a chapter never starts from the previous one's value.
    this.progress = this.target;
    void this.prepare(id).then(() => {
      if (this.active !== chapter) return;
      chapter.group.visible = true;
      chapter.enter(this.ctx);
      this.activeReady = true;
    });
  }

  /** Initialise a chapter once. Safe to call many times. */
  prepare(id: ChapterId): Promise<void> {
    let p = this.ready.get(id);
    if (!p) {
      const chapter = this.chapters.get(id)!;
      p = Promise.resolve(chapter.init(this.ctx)).catch((error) => {
        console.error(`chapter ${id} failed to init`, error);
      });
      this.ready.set(id, p);
    }
    return p;
  }

  /** Initialise every chapter in order, after the first one is up, without blocking the frame loop. */
  preloadAll(order: ChapterId[]) {
    let chain: Promise<void> = Promise.resolve();
    for (const id of order) {
      chain = chain.then(
        () =>
          new Promise<void>((resolve) => {
            const idle = (window as Window & { requestIdleCallback?: (cb: () => void) => number })
              .requestIdleCallback;
            const run = () => void this.prepare(id).then(resolve);
            if (idle) idle(run);
            else setTimeout(run, 50);
          }),
      );
    }
    return chain;
  }

  start() {
    if (this.running) return;
    this.running = true;
    this.last = performance.now();
    const loop = () => {
      this.raf = requestAnimationFrame(loop);
      this.tick();
    };
    this.raf = requestAnimationFrame(loop);
  }

  stop() {
    this.running = false;
    cancelAnimationFrame(this.raf);
  }

  get currentProgress() {
    return this.progress;
  }

  get currentChapter() {
    return this.active?.id ?? null;
  }

  /** Render one frame. Public so the lab can step without the loop. */
  tick() {
    const now = performance.now();
    const dt = Math.min((now - this.last) / 1000, 1 / 20);
    this.last = now;
    this.time += dt;
    const t = this.time;

    // Pointer smoothing and velocity.
    const k = 1 - Math.exp(-dt * 6);
    const px = this.pointer.x + (this.rawPointer.x - this.pointer.x) * k;
    const py = this.pointer.y + (this.rawPointer.y - this.pointer.y) * k;
    this.pointer.vx = dt > 0 ? (px - this.pointer.x) / dt : 0;
    this.pointer.vy = dt > 0 ? (py - this.pointer.y) / dt : 0;
    this.pointer.x = px;
    this.pointer.y = py;

    // Progress eases toward the scroll target so a fast scroll still reads as motion.
    this.progress += (this.target - this.progress) * (1 - Math.exp(-dt * 7));

    // Background and fog ease.
    const bg = this.scene.background as THREE.Color;
    bg.lerp(this.bgTarget, 1 - Math.exp(-dt * 3));
    if (this.scene.fog) {
      const fog = this.scene.fog as THREE.Fog;
      fog.color.copy(bg);
      if (this.fogTarget) {
        fog.near += (this.fogTarget.near - fog.near) * (1 - Math.exp(-dt * 3));
        fog.far += (this.fogTarget.far - fog.far) * (1 - Math.exp(-dt * 3));
      } else {
        fog.near += (200 - fog.near) * (1 - Math.exp(-dt * 3));
        fog.far += (400 - fog.far) * (1 - Math.exp(-dt * 3));
      }
    }

    if (this.active && this.activeReady) {
      this.active.update(this.ctx, this.progress, dt, t);
    }

    this.floor.update(dt, t, this.camera);

    // Shake: decaying noise added to the camera position for this render only.
    if (this.shakeAmp > 0.001) {
      const a = this.shakeAmp * 0.12;
      const s = this.shakeSeed + t * 38;
      this.shakeOffset.set(
        Math.sin(s * 1.3) * a,
        Math.sin(s * 1.7 + 1.0) * a * 0.8,
        Math.sin(s * 0.9 + 2.0) * a * 0.4,
      );
      this.camera.position.add(this.shakeOffset);
      this.shakeAmp *= Math.exp(-dt * 3.2);
    } else {
      this.shakeOffset.set(0, 0, 0);
      this.shakeAmp = 0;
    }

    this.renderer.render(this.scene, this.camera);
    this.camera.position.sub(this.shakeOffset);
  }

  resize() {
    const canvas = this.renderer.domElement;
    const width = canvas.clientWidth || window.innerWidth;
    const height = canvas.clientHeight || window.innerHeight;
    const mobile = width < 768;
    const dpr = Math.min(window.devicePixelRatio || 1, mobile ? 1.5 : 2) * this.renderScale;
    this.renderer.setPixelRatio(dpr);
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    Object.assign(this.viewport, { width, height, dpr, aspect: width / height, mobile });
    for (const chapter of this.chapters.values()) chapter.resize?.(this.ctx);
  }

  /** Drive the pointer from code (the lab and tests). */
  setPointer(x: number, y: number) {
    this.rawPointer.x = x;
    this.rawPointer.y = y;
  }

  private bindPointer(canvas: HTMLCanvasElement) {
    const target = canvas.ownerDocument;
    const onMove = (e: PointerEvent) => {
      if (e.pointerType === "touch") return;
      this.rawPointer.x = (e.clientX / window.innerWidth) * 2 - 1;
      this.rawPointer.y = (e.clientY / window.innerHeight) * 2 - 1;
    };
    const onDown = () => (this.pointer.down = true);
    const onUp = () => (this.pointer.down = false);
    const onLeave = () => {
      this.rawPointer.x = 0;
      this.rawPointer.y = 0;
    };
    target.addEventListener("pointermove", onMove, { passive: true });
    target.addEventListener("pointerdown", onDown, { passive: true });
    target.addEventListener("pointerup", onUp, { passive: true });
    target.documentElement.addEventListener("mouseleave", onLeave);
    this.cleanups.push(() => {
      target.removeEventListener("pointermove", onMove);
      target.removeEventListener("pointerdown", onDown);
      target.removeEventListener("pointerup", onUp);
      target.documentElement.removeEventListener("mouseleave", onLeave);
    });
  }

  dispose() {
    this.stop();
    for (const fn of this.cleanups) fn();
    for (const chapter of this.chapters.values()) chapter.dispose();
    this.floor.dispose();
    this.renderer.dispose();
  }
}
