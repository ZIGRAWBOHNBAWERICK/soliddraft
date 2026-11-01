import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useRef } from "react";

import { about, asset, captions, features, get, hero, rail, sections, site, statement } from "@/content";

export const Route = createFileRoute("/")({
  component: Index,
});

/** The mark: an isometric block with its front vertex, the first thing SolidDraft draws. */
function Monogram() {
  return (
    <svg aria-hidden="true" viewBox="0 0 64 64">
      <path
        d="M32 8 54 20v24L32 56 10 44V20z"
        fill="none"
        stroke="currentColor"
        strokeLinejoin="round"
        strokeWidth="4"
      />
      <path d="M32 32 54 20M32 32v24M32 32 10 20" fill="none" stroke="currentColor" strokeWidth="4" />
      <circle className="monogram__dot" cx="32" cy="32" r="4.2" />
    </svg>
  );
}

function Index() {
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    let dispose: (() => void) | undefined;
    let cancelled = false;
    void import("@/night/boot").then(({ boot }) => {
      if (!cancelled) dispose = boot(root);
    });
    // Cards: loop the clip on hover, never download it before that.
    const onOver = (e: Event) => {
      const video = (e.target as HTMLElement).closest(".card")?.querySelector("video");
      if (video) void video.play().catch(() => undefined);
    };
    const onOut = (e: Event) => {
      const video = (e.target as HTMLElement).closest(".card")?.querySelector("video");
      if (video) video.pause();
    };
    root.addEventListener("pointerover", onOver);
    root.addEventListener("pointerout", onOut);
    // The statement's background clip loads only when its section comes near, and plays only in view.
    const lazy = root.querySelector<HTMLVideoElement>("video[data-lazy-src]");
    let observer: IntersectionObserver | undefined;
    if (lazy && !matchMedia("(prefers-reduced-motion: reduce)").matches) {
      observer = new IntersectionObserver(
        (entries) => {
          for (const entry of entries) {
            if (entry.isIntersecting) {
              if (!lazy.src) lazy.src = lazy.dataset.lazySrc ?? "";
              void lazy.play().catch(() => undefined);
            } else {
              lazy.pause();
            }
          }
        },
        { rootMargin: "60% 0px" },
      );
      observer.observe(lazy);
    }
    return () => {
      cancelled = true;
      dispose?.();
      observer?.disconnect();
      root.removeEventListener("pointerover", onOver);
      root.removeEventListener("pointerout", onOut);
    };
  }, []);

  return (
    <div className="page" ref={rootRef}>
      <canvas aria-hidden="true" className="stage" />
      <div aria-hidden="true" className="veil" />

      <header className="topbar">
        <a aria-label="SolidDraft home" className="wordmark" href="#intro">
          <Monogram />
          <span>{site.name}</span>
        </a>
        <a className="pill" href={site.links.app} rel="noreferrer" target="_blank">
          <span>Open the app</span>
        </a>
      </header>

      <nav aria-label="Sections" className="rail">
        <ul>
          {rail.map((item) => (
            <li key={item.id}>
              <a data-nav={item.label} data-section={item.id} href={`#${item.id}`}>
                <i aria-hidden="true" />
                <span>{item.label}</span>
              </a>
            </li>
          ))}
        </ul>
      </nav>

      <div className="hud">
        <div className="caption caption--hero" data-caption data-chapter="hero" data-from="0" data-to="0.55">
          <p className="kicker">{hero.kicker}</p>
          <h1 className="display">{hero.title}</h1>
          <p className="caption__line">{hero.line}</p>
          <ul className="hero__actions">
            {hero.actions.map((a) => (
              <li key={a.href}>
                <a className={a.primary ? "pill pill--solid" : "pill"} href={a.href} rel="noreferrer" target="_blank">
                  <span>{a.label}</span>
                </a>
              </li>
            ))}
          </ul>
        </div>
        {captions.map((c, i) => (
          <div
            className="caption"
            data-caption
            data-chapter={c.chapter}
            data-from={c.from}
            data-to={c.to}
            key={`${c.chapter}-${i}`}
          >
            <h2 className="caption__title">{c.title}</h2>
            <p className="caption__line">{c.line}</p>
            {c.items && (
              <ul className="caption__items">
                {c.items.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ul>
            )}
          </div>
        ))}
        <div className="controls">
          <span className="control control--scroll">
            <span>Scroll</span>
            <i aria-hidden="true" className="control__line" />
          </span>
          <a className="control" data-skip href="#kernel">
            <span>Skip intro</span>
            <i aria-hidden="true" className="control__arrow" />
          </a>
          <button aria-pressed="false" className="control control--sound" data-sound type="button">
            <i aria-hidden="true" className="control__dot" />
            <span data-sound-label>Sound off</span>
          </button>
        </div>
      </div>

      <main>
        {sections.map((s) => {
          const style = s.flow ? undefined : { height: `${s.span}vh` };
          const common = {
            "data-chapter": s.chapter,
            "data-nav": s.nav,
            "data-section": "",
            "data-theme": s.theme,
            "data-veil": s.veil ? "true" : undefined,
            id: s.id,
            style,
          } as const;
          if (s.id === "statement") {
            return (
              <section className="section section--statement" key={s.id} {...common}>
                <div className="statement">
                  <video
                    aria-hidden="true"
                    className="statement__video"
                    data-lazy-src={asset("assets/world/bracket-drift.mp4")}
                    loop
                    muted
                    playsInline
                    poster={asset("assets/world/bracket-drift.webp")}
                    preload="none"
                  />
                  <p className="kicker">{statement.kicker}</p>
                  <h2 className="display display--xl">
                    {statement.title.map((line) => (
                      <span key={line}>{line}</span>
                    ))}
                  </h2>
                  <a className="pill pill--big" href={statement.cta.href} rel="noreferrer" target="_blank">
                    <span>{statement.cta.label}</span>
                    <i aria-hidden="true" className="control__arrow" />
                  </a>
                </div>
              </section>
            );
          }
          if (s.id === "features") {
            return (
              <section className="section section--what" key={s.id} {...common}>
                <div className="what">
                  <div>
                    <h2 className="display">{features.title}</h2>
                    <p className="what__intro">{features.line}</p>
                  </div>
                  <ul className="what__list">
                    {features.items.map((item) => (
                      <li key={item.name}>
                        <a href={item.href} rel="noreferrer" target="_blank">
                          <span className={item.inDesign ? "kicker what__status" : "kicker"}>{item.kind}</span>
                          <span className="what__name">{item.name}</span>
                          <span className="what__line">{item.line}</span>
                        </a>
                      </li>
                    ))}
                  </ul>
                </div>
              </section>
            );
          }
          if (s.id === "get") {
            return (
              <section className="section section--work" key={s.id} {...common}>
                <div className="work">
                  <h2 className="display">Get SolidDraft</h2>
                  <ul className="cards">
                    {get.map((item) => (
                      <li key={item.id}>
                        <a className="card" href={item.href} rel="noreferrer" target="_blank">
                          <span className="card__media">
                            {item.video ? (
                              <video loop muted playsInline poster={item.poster} preload="none" src={item.video} />
                            ) : (
                              <img alt={item.alt} loading="lazy" src={item.poster} />
                            )}
                          </span>
                          <span className="card__tags">
                            {item.tags.map((tag) => (
                              <span key={tag}>{tag}</span>
                            ))}
                            {item.status && <span className="card__status">{item.status}</span>}
                          </span>
                          <span className="card__title">{item.title}</span>
                          <span className="card__blurb">{item.blurb}</span>
                        </a>
                      </li>
                    ))}
                  </ul>
                </div>
              </section>
            );
          }
          if (s.id === "about") {
            return (
              <section className="section section--contact" key={s.id} {...common}>
                <div className="contact">
                  <h2 className="display display--xl">{about.title}</h2>
                  <p className="contact__line">{about.line}</p>
                  <ul className="support">
                    {about.support.map((b) => (
                      <li key={b.href}>
                        <a className={b.primary ? "pill pill--big pill--solid" : "pill pill--big"} href={b.href} rel="noreferrer" target="_blank">
                          <span>{b.label}</span>
                          <i aria-hidden="true" className="control__arrow" />
                        </a>
                      </li>
                    ))}
                  </ul>
                  <ul className="contact__links">
                    {about.links.map((link) => (
                      <li key={link.href}>
                        <a href={link.href} rel="noreferrer" target="_blank">
                          {link.label}
                        </a>
                      </li>
                    ))}
                  </ul>
                </div>
                <footer className="foot">
                  <span>{about.foot.credit}</span>
                  <span>
                    {about.foot.company}{" "}
                    <a href={about.foot.other.href} rel="noreferrer" target="_blank">
                      {about.foot.other.label}
                    </a>
                    .
                  </span>
                </footer>
              </section>
            );
          }
          return <section className="section section--span" key={s.id} {...common} />;
        })}
      </main>
    </div>
  );
}
