import { expect, test } from "bun:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { GalleryMedia } from "../src/components/gallery/gallery-media";
import { JustifiedLayoutEngine } from "../src/components/gallery/justified-engine";
import type { GalleryItem } from "../src/components/gallery/types";

function makeItems(count: number): GalleryItem[] {
  return Array.from({ length: count }, (_, index) => ({
    id: `item-${index}`,
    kind: "image",
    status: "ready",
    src: `/images/${index}.webp`,
    width: 1200,
    height: 800,
    prompt: `Image ${index}`,
    alt: `Image ${index}`,
    groupId: "today",
    groupLabel: "Today",
  }));
}

test("history layout returns only the requested row window", () => {
  const engine = new JustifiedLayoutEngine();
  engine.setItems(makeItems(500));
  engine.setConfig({ containerWidth: 1200, targetRowHeight: 220, gap: 6, grouped: false });

  const layout = engine.getLayout();
  const firstWindow = engine.getWindow(0, 800, 400);
  const laterWindow = engine.getWindow(layout.totalHeight / 2, 800, 400);

  expect(firstWindow.startRow).toBe(0);
  expect(firstWindow.endRow).toBeLessThan(layout.rows.length);
  expect(laterWindow.startRow).toBeGreaterThan(0);
  expect(laterWindow.endRow - laterWindow.startRow).toBeLessThan(layout.rows.length);
});

test("history video stays idle until interaction", () => {
  const item: GalleryItem = {
    id: "video-1",
    kind: "video",
    status: "ready",
    src: "/images/video-poster.webp",
    videoSrc: "/videos/video-1.mp4",
    width: 1920,
    height: 1080,
    prompt: "Video generation",
    alt: "Video poster",
    groupId: "today",
    groupLabel: "Today",
  };
  const html = renderToStaticMarkup(
    createElement(GalleryMedia, {
      item,
      tier: "full",
      playing: false,
      reducedMotion: false,
    }),
  );

  expect(html).toContain('<video class="absolute inset-0 size-full object-cover"');
  expect(html).toContain('preload="none"');
  expect(html).toContain('<source src="/videos/video-1.mp4" type="video/mp4"/>');
});
