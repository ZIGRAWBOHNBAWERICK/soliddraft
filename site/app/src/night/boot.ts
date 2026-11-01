/**
 * Boot the live page: WebGL stage, scroll, captions, rail, sound and controls.
 * Without WebGL or with reduced motion the page stays static (html.is-static) and readable.
 */
import { CHAPTER_ORDER, createChapters } from "./chapters";
import { initScroll } from "./scroll";
import { createSound } from "./sound";
import { Stage } from "./stage";

export function hasWebGL(): boolean {
  try {
    const canvas = document.createElement("canvas");
    return Boolean(canvas.getContext("webgl2") ?? canvas.getContext("webgl"));
  } catch {
    return false;
  }
}

export function boot(root: HTMLElement): () => void {
  const html = document.documentElement;
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const canvas = root.querySelector<HTMLCanvasElement>("canvas.stage");
  if (reduced || !canvas || !hasWebGL()) {
    html.classList.add("is-static");
    return () => html.classList.remove("is-static");
  }

  const sound = createSound();
  const mobile = window.innerWidth < 768 || matchMedia("(pointer: coarse)").matches;
  const stage = new Stage({
    canvas,
    chapters: createChapters(),
    quality: mobile ? 0 : 2,
    onAccent: (color) => html.style.setProperty("--accent", color),
    cue: (name) => sound.cue(name),
  });

  html.classList.add("is-loading");
  const rail = [...root.querySelectorAll<HTMLAnchorElement>(".rail a[data-section]")];
  const scroll = initScroll({
    root,
    stage,
    onSection: (id) => {
      const section = root.querySelector<HTMLElement>(`#${CSS.escape(id)}`);
      const nav = section?.dataset.nav;
      for (const a of rail) a.classList.toggle("is-active", a.dataset.nav === nav);
      sound.cue("tick");
    },
  });

  // Controls.
  const soundButton = root.querySelector<HTMLButtonElement>("[data-sound]");
  const onSound = async () => {
    const on = await sound.toggle();
    soundButton?.setAttribute("aria-pressed", String(on));
    const label = soundButton?.querySelector("[data-sound-label]");
    if (label) label.textContent = on ? "Sound on" : "Sound off";
  };
  soundButton?.addEventListener("click", onSound);

  const skip = root.querySelector<HTMLAnchorElement>("[data-skip]");
  const onSkip = (e: Event) => {
    e.preventDefault();
    scroll.scrollTo("#kernel", 1.8);
  };
  skip?.addEventListener("click", onSkip);

  let disposed = false;
  void stage.prepare("hero").then(() => {
    if (disposed) return;
    html.classList.remove("is-loading");
    html.classList.add("is-live");
    stage.start();
    stage.resize();
    void stage.preloadAll(CHAPTER_ORDER);
  });

  return () => {
    disposed = true;
    soundButton?.removeEventListener("click", onSound);
    skip?.removeEventListener("click", onSkip);
    scroll.destroy();
    stage.dispose();
    sound.dispose();
    html.classList.remove("is-live", "is-loading", "is-light", "is-veiled", "is-end");
    html.style.removeProperty("--accent");
  };
}
