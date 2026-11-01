# Three-pass verification

Base: local main at `460bb2fac49935891839403757f43da0c87b1d5d`.
Branch: `fm/zbb-site-impeccable-three-pass`.
Target: `app/src/routes/index.tsx`, exercised through the static artifact at `http://127.0.0.1:4273/`.

## Scope and results

Exactly three critiques are archived in `critique/`. Each records its design assessment before that pass's detector output. No subagent tool was exposed, so all three disclose the skill's sequential, single-context fallback. No Fable model was used.

| Pass | Selected problem | Correction | Evidence |
| --- | --- | --- | --- |
| 1 | Product copy overlaps the header on short screens | Height-based composition, with the decorative icon sheet omitted on short screens | `pass-1-before-landscape-product.png`, `pass-1-after-landscape-product.png`, small-phone captures |
| 2 | Resolution words remain clipped after a chapter jump | Drive the rectangular mask from chapter entry, finishing before the chapter pins | `pass-2-before-*-resolution.png`, `pass-2-after-*-resolution.png`, mask geometry |
| 3 | Copy backdrop hides chapter navigation | Separate sticky navigation layer above the story, opaque control backgrounds and readable label color | `pass-3-before-*-product.png`, `pass-3-after-*-product.png` |

Source changes are confined to the scroll-scrub composition and CSS. No controller logic, factual copy, route, palette, font or media asset changed.

## Checks

- `cd app && bun run lint`: passed.
- `cd app && bun run test`: five tests passed, 17 assertions.
- `cd app && bun run build:pages`: passed, including the UI contract and TypeScript checks.
- Prettier check on both changed source files: passed.
- Final Impeccable CLI scan of the homepage and scroll-scrub component: zero findings.
- Browser detector injection succeeded in each pass. It reported the existing closing CareerForge label above the heading. That style preference was not selected over the reproduced usability faults.
- Browser checks passed at 1440x900, 390x844, 320x568 and 844x390: product heading below the header, product copy above navigation, chapter controls receive pointer input, inactive-label contrast 7.60:1, keyboard activation and focus visibility, complete resolution heading after jumps.
- Reduced motion rendered no video and no heading mask. A missing route rendered the existing not-found page.

The browser wrapper twice lost selection during the second pass. Failed attempts are not evidence. Firstmate authorized the direct-browser fallback; fresh Puppeteer captures and checks replaced those attempts. This was a tool recovery within pass 2, not another critique.

## Review and remaining scope

Lavish review: http://blade-omarchy.carp-trout.ts.net:4387/session/d9631893524fb635

The review uses the site's own palette and self-hosted Satoshi fonts. Its rendered page was checked. It remains available; no live feedback monitoring is claimed.

No unresolved product or visual-direction decision needs the captain's judgment. The review says so instead of inventing decision cards. The mobile rail still scrolls horizontally; its final label may be partially outside the viewport until swiped or focused. This minor discoverability issue remains outside the three selected corrections.

No unmerged code was deployed. Merge and the scoped-credential Cloudflare production cutover remain separate operations.

## Evidence and rerun

Task evidence root:
`/home/bohn/labs/axi-sandbox/firstmate/data/zbb-site-impeccable-three-pass/`

It contains the frozen assessments, before/after screenshots, detector logs, build logs, `final-browser-check.json`, and the Lavish artifact in `review/`. The runnable `check-mask.mjs` and `verify-final.mjs` use the installed Puppeteer package without adding a project dependency.

To repeat those browser checks, build the Pages artifact, serve it with `cd app && PORT=4273 node scripts/serve-static.mjs`, launch isolated Chromium with remote debugging on port 9337, and run the evidence scripts with Node. The temporary site and task browser were stopped after verification. The Lavish review stays available separately.
