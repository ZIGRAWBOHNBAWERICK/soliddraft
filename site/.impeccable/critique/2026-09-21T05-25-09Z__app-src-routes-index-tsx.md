---
target: landing
total_score: 21
max_score: 32
na_heuristics: 7,10
p0_count: 0
p1_count: 2
target_identity: "file:/home/bohn/labs/axi-sandbox/firstmate/projects/zigrawbohnbawerick-org/app/src/routes/index.tsx"
target_fingerprint: "sha256:46dd1bdeabcb45a061c180c18bc791d61414beee9285c87d9de182d0a89d28ae"
target_path: /home/bohn/labs/axi-sandbox/firstmate/projects/zigrawbohnbawerick-org/app/src/routes/index.tsx
timestamp: 2026-09-21T05-25-09Z
slug: app-src-routes-index-tsx
---
# Critique: landing (app/src/routes/index.tsx)

Method: dual-agent (A: design review · B: detector + browser)

## Design health score
| # | Heuristic | Score | Key issue |
|---|---|---|---|
| 1 | Visibility of system status | 3 | Rail and progress bar work; rail lags a chapter during the Resolution mask |
| 2 | Match system / real world | 2 | Rail labels Opening/Principle/Resolution are internal words |
| 3 | User control and freedom | 3 | Rail jumps; ~6.6k px scroll length is fixed by the film |
| 4 | Consistency and standards | 2 | Three CareerForge links to one anchor; iOS/Web chips look like buttons but are inert |
| 5 | Error prevention | 3 | Little to get wrong; chips invite dead taps |
| 6 | Recognition rather than recall | 3 | Short copy, repeated nouns |
| 7 | Flexibility and efficiency | n/a | One-page brochure |
| 8 | Aesthetic and minimalist design | 3 | Restrained, but headlines slide under the transparent header |
| 9 | Error recovery | 2 | "Coming soon" offers no next step |
| 10 | Help and documentation | n/a | Landing page |
| **Total** | | **21/32** | **Acceptable (66%)** |

## Design specificity
Stone-strata film, rose on blue-charcoal, and the records-into-two-documents metaphor are authored for CareerForge. The chapter-rail scaffold and big-headline-over-film stacking are template defaults. Detector: CLI 0 findings; browser 1 (kicker-above-heading, index.tsx:40-41, "CareerForge" label over "Coming soon.").

## Priority issues
1. [P1] Dead-end ending: no waitlist, contact, ABN, or privacy link after "Coming soon." Fix: waitlist or mailto in closing panel; contact, ABN, privacy in footer (index.tsx:39-48).
2. [P1] Headlines scroll behind and overlap the transparent fixed header on desktop and mobile. Fix: scrimmed header background or clip story layer below header height (site.css:74-86).
3. [P2] Resolution clip-path mask leaves sliced glyphs ("ur recor / ought / gether") over a long scroll range. Fix: shorten range or fade instead of slice (scroll-scrub.css:242-252).
4. [P2] Rail: 5 internal labels, "Resolution" truncates at 390px, inactive labels low contrast at 0.72rem. Fix: visitor-facing names or numbered ticks, 4 items, higher contrast (scroll-scrub-scenes.ts:189-224, scroll-scrub.css:121-134).
5. [P3] Three identical CareerForge links; header link duplicates hero CTA. Fix: give header link a distinct job (waitlist/contact) (index.tsx:26, scroll-scrub-scenes.ts:182).

## Persona red flags
- Jordan: what CareerForge does is not stated until chapter four; hero body leads with legal wording.
- Riley: fast scrub lands on half-masked title and stale rail state; focus drops to body after anchor Enter; no contact to verify the business.
- Casey: "Resolution" clipped; header CTA hidden on mobile; ~6.2k px to the end.

## Minor observations
- "One / job. Two / documents." breaks badly at 1440px; use text-wrap: balance.
- Mobile icon sheet slides under the header.
- landing-content.ts holds unused template copy ("Open app").
- 0.72rem rose kickers are small.
- Keyboard: 7 tab stops before the hero CTA.

## Questions
1. What should a convinced visitor do in the next 10 seconds?
2. Does a five-chapter film earn its length for one "coming soon" message?
3. Is "We build for work." this company's promise or any studio's?
