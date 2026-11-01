import { describe, expect, test } from "bun:test";
import { createElement } from "react";
import type { ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Route as landingRoute } from "../src/routes/index";
import { site } from "../src/content";

describe("SolidDraft landing contract", () => {
  test("renders the public landing route", () => {
    const Component = landingRoute.options.component as ComponentType;
    const html = renderToStaticMarkup(createElement(Component));

    expect(html).toContain('aria-label="SolidDraft home"');
    for (const id of ["intro", "kernel", "model", "drawings", "anywhere", "features", "get", "about"]) {
      expect(html).toContain(`id="${id}"`);
    }
    expect(html).toContain("SolidDraft");
    expect(html).toContain("Catallax");
    // The clear button to the app, the downloads and the repository.
    expect(html).toContain(`href="${site.links.app}"`);
    expect(html).toContain(`href="${site.links.releases}"`);
    expect(html).toContain(`href="${site.links.repo}"`);
    // The company's credit and licence line; no personal name, nothing about other work.
    expect(html).toContain("Made by Zigrawbohnbawerick Pty Ltd. MIT licence.");
    // The donation buttons stay at their addresses; the Ko-fi address is the one allowed exception to the name search.
    expect(html).toContain(`href="${site.links.kofi}"`);
    expect(html).toContain(`href="${site.links.sponsors}"`);
    expect(html).toContain("Support on Ko-fi");
    expect(html).toContain("GitHub Sponsors");
    expect(html.replaceAll(site.links.kofi, "")).not.toMatch(/Pedro|De Paiva/i);
    expect(html).not.toContain("CareerForge");
    expect(html).not.toContain("MRB");
    // The app opens beside the page on GitHub Pages.
    expect(site.links.app).toBe("https://zigrawbohnbawerick.github.io/soliddraft/app/");
    // Unbuilt features only appear with the in-design label.
    expect(html).toContain("In design");
    expect(html).not.toMatch(/<[^>]*>[^<]*—/);
    expect(html).toContain('preload="none"');
  });
});
