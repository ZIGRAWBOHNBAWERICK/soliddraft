/**
 * Scroll to chapter progress. Lenis smooth scroll bridged to GSAP ScrollTrigger; each page
 * section with data-chapter drives the stage while it is in view. Captions, the rail marker,
 * the light theme and the veil follow the same triggers.
 */
import gsap from "gsap";
import { ScrollTrigger } from "gsap/ScrollTrigger";
import Lenis from "lenis";

import type { Stage } from "./stage";
import type { ChapterId } from "./types";

gsap.registerPlugin(ScrollTrigger);

export interface ScrollOptions {
  root: HTMLElement;
  stage: Stage;
  onSection?: (id: string) => void;
}

export interface ScrollHandle {
  scrollTo(target: string | number, duration?: number): void;
  destroy(): void;
}

/** Show the captions of `chapter` whose [from, to] contains `progress`; hide the rest. Pure DOM, reused by the lab. */
export function updateCaptions(root: ParentNode, chapter: ChapterId | null, progress: number) {
  for (const el of root.querySelectorAll<HTMLElement>("[data-caption]")) {
    const on =
      el.dataset.chapter === chapter &&
      progress >= Number(el.dataset.from) &&
      progress <= Number(el.dataset.to);
    if (el.classList.contains("is-on") !== on) el.classList.toggle("is-on", on);
  }
}

export function initScroll({ root, stage, onSection }: ScrollOptions): ScrollHandle {
  const html = document.documentElement;
  const lenis = new Lenis({ lerp: 0.09, smoothWheel: true });
  if (import.meta.env.DEV) {
    (window as unknown as { __st?: typeof ScrollTrigger; __lenis?: Lenis }).__st = ScrollTrigger;
    (window as unknown as { __st?: typeof ScrollTrigger; __lenis?: Lenis }).__lenis = lenis;
  }
  lenis.on("scroll", ScrollTrigger.update);
  const raf = (time: number) => lenis.raf(time * 1000);
  gsap.ticker.add(raf);
  gsap.ticker.lagSmoothing(0);

  const triggers: ScrollTrigger[] = [];
  const sections = [...root.querySelectorAll<HTMLElement>("section[data-section]")];
  const active = new Set<HTMLElement>();
  let current: HTMLElement | null = null;
  let lastChapter: ChapterId | null = null;
  let lastProgress = -1;

  const apply = (section: HTMLElement, progress: number) => {
    const chapter = (section.dataset.chapter as ChapterId | undefined) ?? null;
    if (!chapter) {
      // A text-only section: the last chapter stays on the canvas, its captions go away.
      if (lastChapter !== null) {
        updateCaptions(root, null, 0);
        lastChapter = null;
      }
      return;
    }
    if (chapter) {
      stage.setActive(chapter, progress);
      const p = stage.currentProgress;
      if (chapter !== lastChapter || Math.abs(p - lastProgress) > 0.002) {
        updateCaptions(root, chapter, p);
        lastChapter = chapter;
        lastProgress = p;
      }
    }
  };

  const activate = (section: HTMLElement, progress: number) => {
    current = section;
    html.classList.toggle("is-light", section.dataset.theme === "light");
    html.classList.toggle("is-veiled", section.dataset.veil === "true");
    html.classList.toggle("is-end", section === sections[sections.length - 1]);
    onSection?.(section.id);
    apply(section, progress);
  };

  const last = sections[sections.length - 1];
  for (const section of sections) {
    triggers.push(
      ScrollTrigger.create({
        trigger: section,
        // The last section ends at the page bottom, so "top top" would never fire for it:
        // it takes over once its top passes two thirds of the viewport.
        start: section === last ? "top 66%" : "top top",
        end: "bottom bottom",
        onUpdate: (self) => {
          if (current === section) apply(section, self.progress);
        },
        onToggle: (self) => {
          if (self.isActive) active.add(section);
          else active.delete(section);
          // The deepest active section owns the page, whichever way the visitor scrolls.
          let next: HTMLElement | null = null;
          for (const s of sections) if (active.has(s)) next = s;
          if (next && next !== current) {
            const t = triggers.find((x) => x.trigger === next);
            activate(next, t?.progress ?? 0);
          }
        },
      }),
    );
  }

  // Captions ease with the stage's smoothed progress, so refresh them on the GSAP ticker too.
  // The same tick settles ownership when no trigger is active (between a section's end and the
  // next one's start, or after an instant jump to the very bottom): the section under the
  // viewport top owns the page.
  const tickCaptions = () => {
    if (active.size === 0) {
      const top = window.scrollY;
      const hit =
        sections.find((s) => s.offsetTop <= top && s.offsetTop + s.offsetHeight > top) ??
        sections[sections.length - 1];
      if (hit !== current) {
        const span = Math.max(1, hit.offsetHeight - window.innerHeight);
        activate(hit, Math.min(1, Math.max(0, (top - hit.offsetTop) / span)));
      }
    }
    if (!current?.dataset.chapter) return;
    const p = stage.currentProgress;
    if (Math.abs(p - lastProgress) > 0.002) {
      updateCaptions(root, current.dataset.chapter as ChapterId, p);
      lastProgress = p;
    }
  };
  gsap.ticker.add(tickCaptions);

  // Anchor links scroll through Lenis.
  const onClick = (event: Event) => {
    const link = (event.target as HTMLElement).closest<HTMLAnchorElement>('a[href^="#"]');
    if (!link) return;
    const id = link.getAttribute("href")!.slice(1);
    const el = document.getElementById(id);
    if (!el) return;
    event.preventDefault();
    lenis.scrollTo(el, { duration: 1.6, offset: 0 });
  };
  root.addEventListener("click", onClick);

  // First paint: pick the section under the top of the viewport.
  requestAnimationFrame(() => {
    ScrollTrigger.refresh();
    const top = window.scrollY;
    const hit = sections.find((s) => s.offsetTop <= top && s.offsetTop + s.offsetHeight > top) ?? sections[0];
    if (hit && !current) {
      const span = Math.max(1, hit.offsetHeight - window.innerHeight);
      activate(hit, Math.min(1, Math.max(0, (top - hit.offsetTop) / span)));
    }
  });

  return {
    scrollTo(target, duration = 1.6) {
      lenis.scrollTo(target, { duration, offset: 0 });
    },
    destroy() {
      root.removeEventListener("click", onClick);
      gsap.ticker.remove(tickCaptions);
      gsap.ticker.remove(raf);
      for (const t of triggers) t.kill();
      lenis.destroy();
    },
  };
}
