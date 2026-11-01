# SolidDraft site

This folder is the public website for SolidDraft. The page lives in `app/`; `app/AGENTS.md` maps
the files and `app/design-brief.md` is the design contract.

- Every part on the page is a real SolidDraft export. `parts/` holds the run scripts and saved
  parts; `tools/blender/` turns them into GLBs, stills and turntables with headless Blender.
- Every picture of the app is a real screenshot. `tools/blender/sd_shots.sh` has the commands.
- Generated atmosphere media (Higgsfield) is logged in `refs/higgsfield-manifest.jsonl`.
- Run `( cd app && bun run build:pages )` for the static artifact in `app/dist/pages`.
- Run `( cd app && bun run lint && bun run test )` before delivery. `build:pages` includes the
  UI check and the TypeScript check.
- Hosting: `.github/workflows/pages.yml` builds this page under `/soliddraft/`, adds the released
  browser zip under `app/` and deploys to GitHub Pages when the repository variable
  `SOLIDDRAFT_PAGES_ENABLED` is `true`. `release.yml` calls it after a tag publishes. The public
  address and the app's address are one setting each in `app/src/content.ts`.

## Maintaining this file

Keep this file for knowledge useful to almost every future agent session in this folder.
Do not repeat what the codebase already shows; point to the authoritative file or command instead.
Prefer rewriting or pruning existing entries over appending new ones.
