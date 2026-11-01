import type { Chapter, ChapterId } from "../types";

import { createCad } from "./cad";
import { createDraw } from "./draw";
import { createHero } from "./hero";
import { createKernel } from "./kernel";
import { createLight } from "./light";
import { createScreen } from "./screen";
import { createSmoke } from "./smoke";

/** Scroll order. The page sections carry the same ids in data-chapter. */
export const CHAPTER_ORDER: ChapterId[] = ["hero", "kernel", "cad", "draw", "screen", "light", "smoke"];

export function createChapters(only?: ChapterId[]): Chapter[] {
  const all: Record<ChapterId, () => Chapter> = {
    hero: createHero,
    kernel: createKernel,
    cad: createCad,
    draw: createDraw,
    screen: createScreen,
    light: createLight,
    smoke: createSmoke,
  };
  return (only ?? CHAPTER_ORDER).map((id) => all[id]());
}
