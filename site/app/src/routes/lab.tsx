/**
 * Chapter lab: /lab?c=kernel&p=0.3&q=2
 * One chapter full screen with a progress slider and the page captions, for building and
 * screenshotting a scene on its own. window.__lab exposes set(p), pointer(x, y) and ready.
 */
import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";

import { captions, hero } from "@/content";
import type { ChapterId } from "@/night/types";

export const Route = createFileRoute("/lab")({
  component: Lab,
  validateSearch: (search: Record<string, unknown>) => ({
    c: (typeof search.c === "string" ? search.c : "hero") as ChapterId,
    p: typeof search.p === "number" ? search.p : Number(search.p ?? 0) || 0,
    q: typeof search.q === "number" ? search.q : Number(search.q ?? 2),
    clean: Number(search.clean ?? 0) || 0,
  }),
});

declare global {
  interface Window {
    __lab?: {
      set(p: number): void;
      pointer(x: number, y: number): void;
      ready: Promise<void>;
      chapter: ChapterId;
    };
  }
}

function Lab() {
  const { c, p, q, clean } = Route.useSearch();
  const rootRef = useRef<HTMLDivElement>(null);
  const [progress, setProgress] = useState(p);
  const setRef = useRef<(p: number) => void>(() => undefined);

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    let dispose: (() => void) | undefined;
    let cancelled = false;
    // Load only this chapter's module, so a broken sibling never blocks the lab.
    const modules = import.meta.glob<Record<string, () => import("@/night/types").Chapter>>(
      "../night/chapters/*.ts",
    );
    void Promise.all([import("@/night/stage"), modules[`../night/chapters/${c}.ts`](), import("@/night/scroll")]).then(
      ([{ Stage }, mod, { updateCaptions }]) => {
        if (cancelled) return;
        const factory = Object.values(mod).find((v) => typeof v === "function")!;
        const html = document.documentElement;
        const canvas = root.querySelector<HTMLCanvasElement>("canvas.stage")!;
        const stage = new Stage({
          canvas,
          chapters: [factory()],
          quality: Math.max(0, Math.min(2, q)) as 0 | 1 | 2,
          onAccent: (color) => html.style.setProperty("--accent", color),
        });
        html.classList.add("is-live");
        html.classList.toggle("is-light", c === "light");
        const set = (value: number) => {
          stage.setActive(c, value);
          updateCaptions(root, c, value);
        };
        setRef.current = set;
        const ready = stage.prepare(c).then(() => {
          stage.start();
          stage.resize();
          set(p);
          updateCaptions(root, c, p);
        });
        window.__lab = { set, pointer: (x, y) => stage.setPointer(x, y), ready, chapter: c };
        // Keep captions in step with the eased progress.
        const id = window.setInterval(() => updateCaptions(root, c, stage.currentProgress), 100);
        dispose = () => {
          window.clearInterval(id);
          stage.dispose();
          html.classList.remove("is-live", "is-light");
          delete window.__lab;
        };
      },
    );
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [c, p, q]);

  return (
    <div className="page page--lab" ref={rootRef}>
      <canvas aria-hidden="true" className="stage" />
      <div className="hud" hidden={clean >= 2}>
        <div className="caption caption--hero" data-caption data-chapter="hero" data-from="0" data-to="0.55">
          <p className="kicker">{hero.kicker}</p>
          <h1 className="display">{hero.title}</h1>
          <p className="caption__line">{hero.line}</p>
        </div>
        {captions
          .filter((x) => x.chapter === c)
          .map((x, i) => (
            <div
              className="caption"
              data-caption
              data-chapter={x.chapter}
              data-from={x.from}
              data-to={x.to}
              key={i}
            >
              <h2 className="caption__title">{x.title}</h2>
              <p className="caption__line">{x.line}</p>
              {x.items && (
                <ul className="caption__items">
                  {x.items.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              )}
            </div>
          ))}
      </div>
      {clean ? null : (
      <form className="lab" onSubmit={(e) => e.preventDefault()}>
        <label>
          <span>
            {c} · {progress.toFixed(3)}
          </span>
          <input
            max="1"
            min="0"
            onChange={(e) => {
              const v = Number(e.target.value);
              setProgress(v);
              setRef.current(v);
            }}
            step="0.001"
            type="range"
            value={progress}
          />
        </label>
      </form>
      )}
    </div>
  );
}
