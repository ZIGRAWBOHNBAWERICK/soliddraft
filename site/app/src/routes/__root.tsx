import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  HeadContent,
  Link,
  Outlet,
  Scripts,
  createRootRouteWithContext,
} from "@tanstack/react-router";
import type { ReactNode } from "react";

import appCss from "../site.css?url";
import { asset, site, themeColor } from "../content";

const description = site.description;
const socialImage = `${site.publicUrl}assets/brand/social-card.jpg`;

function NotFoundComponent() {
  return (
    <main className="not-found">
      <img
        alt="A row of machined collars on a shelf, one fallen over"
        className="not-found__media"
        src={asset("assets/brand/not-found.webp")}
      />
      <div className="not-found__content">
        <p className="label">404</p>
        <h1>This part is out of place.</h1>
        <Link className="pill pill--line" to="/">
          <span className="pill__label" data-text="Return home">
            <span>Return home</span>
          </span>
        </Link>
      </div>
    </main>
  );
}

function ErrorComponent({ reset }: { error: Error; reset: () => void }) {
  return (
    <main className="error-page">
      <div className="error-page__content">
        <h1>This page did not load.</h1>
        <p>Try the page again or return home.</p>
        <div className="error-page__actions">
          <button className="pill pill--line" onClick={reset} type="button">
            <span className="pill__label" data-text="Try again">
              <span>Try again</span>
            </span>
          </button>
          <a className="pill pill--line" href={import.meta.env.BASE_URL}>
            <span className="pill__label" data-text="Return home">
              <span>Return home</span>
            </span>
          </a>
        </div>
      </div>
    </main>
  );
}

export const Route = createRootRouteWithContext<{ queryClient: QueryClient }>()({
  head: () => ({
    links: [
      { rel: "stylesheet", href: appCss },
      {
        rel: "preload",
        href: asset("assets/fonts/satoshi-regular.woff2"),
        as: "font",
        type: "font/woff2",
        crossOrigin: "anonymous",
      },
      {
        rel: "preload",
        href: asset("assets/fonts/jetbrains-mono-latin-wght-normal.woff2"),
        as: "font",
        type: "font/woff2",
        crossOrigin: "anonymous",
      },
      { rel: "canonical", href: site.publicUrl },
      { rel: "icon", href: asset("favicon.svg"), type: "image/svg+xml" },
      { rel: "icon", href: asset("favicon.ico"), sizes: "32x32" },
      { rel: "apple-touch-icon", href: asset("apple-touch-icon.png") },
      { rel: "manifest", href: asset("site.webmanifest") },
    ],
    meta: [
      { charSet: "utf-8" },
      { name: "viewport", content: "width=device-width, initial-scale=1" },
      { title: site.title },
      { name: "description", content: description },
      { name: "theme-color", content: themeColor },
      { name: "robots", content: "index, follow" },
      { property: "og:title", content: site.title },
      { property: "og:description", content: description },
      { property: "og:type", content: "website" },
      { property: "og:url", content: site.publicUrl },
      { property: "og:image", content: socialImage },
      { property: "og:image:width", content: "1200" },
      { property: "og:image:height", content: "630" },
      { name: "twitter:card", content: "summary_large_image" },
      { name: "twitter:title", content: site.title },
      { name: "twitter:description", content: description },
      { name: "twitter:image", content: socialImage },
    ],
  }),
  shellComponent: RootShell,
  component: RootComponent,
  notFoundComponent: NotFoundComponent,
  errorComponent: ErrorComponent,
});

function RootShell({ children }: { children: ReactNode }) {
  return (
    <html lang="en" style={{ colorScheme: "dark" }}>
      <head>
        <HeadContent />
      </head>
      <body>
        {children}
        <Scripts />
      </body>
    </html>
  );
}

function RootComponent() {
  const { queryClient } = Route.useRouteContext();

  return (
    <QueryClientProvider client={queryClient}>
      <Outlet />
    </QueryClientProvider>
  );
}
