# Feedback inbox

The site and browser app send feedback to
`https://soliddraft-feedback.sdc-feedback-inbox.workers.dev/feedback`.
The Worker stores accepted messages in the private D1 database `soliddraft-feedback`.
A local sync copies pending rows to issues in the private repository
`ZIGRAWBOHNBAWERICK/soliddraft-feedback`. No GitHub credential is stored in Cloudflare.

The frontend changes ship with the site and the next browser-app release. They do not
replace the currently published app zip.

## Operate the inbox

On the machine with Wrangler OAuth and the local GitHub CLI login, run:

```sh
python3 /absolute/path/to/soliddraft/feedback/sync/sync.py
```

Register that command as a periodic check. It is not scheduled by this repository.
Until it runs, new feedback stays in D1 and does not appear in GitHub. The script needs
`npx`, Python 3 on Linux, `gh-axi`, D1 read/write access and Issues read/write access to
the private feedback repository. It uses Wrangler 4.158.0. It never extracts the local
GitHub token.

Each run checks that the destination repository is private, handles up to 100 pending
rows and prints only IDs and issue URLs. Labels are `source:site` or `source:app`,
`kind:bug|idea|other` and `version:<version>`. Context is a collapsed JSON block.
Message and contact text are fenced to keep mentions and Markdown literal.

A local lock prevents overlapping runs on this machine. A retry finds the issue's
feedback-ID marker before creating another issue, including when issue creation
succeeded but marking the row failed. Do not run sync from multiple machines at once.
Synced rows remain in D1; sync never deletes feedback. Review CLI failures locally and
retry. A failed row stays pending.

`--config /path/to/wrangler.toml` selects another config's `DB` binding for isolated
tests. The GitHub destination stays fixed. `--lock /path/to/lock` selects the local
lock file when needed.

## Deploy

Use an authorized local Wrangler OAuth login. The Worker needs Workers Scripts write,
D1 write and Turnstile widget management permissions. Do not copy the broad local
GitHub token, OAuth credentials or widget secret into source or logs.

```sh
npx wrangler@4.158.0 whoami
npx wrangler@4.158.0 d1 execute DB --remote \
  --config feedback/worker/wrangler.toml --file feedback/worker/schema.sql
npx wrangler@4.158.0 secret put TURNSTILE_SECRET --config feedback/worker/wrangler.toml
npx wrangler@4.158.0 deploy --config feedback/worker/wrangler.toml
```

The database and dedicated non-interactive Turnstile widget already exist. The public
widget key and endpoint live in `client/config.mjs`. The widget permits only
`zigrawbohnbawerick.github.io`. Set its secret using the interactive Wrangler secret
prompt, not a command-line argument. If moving the service, update that config, the
site CSP headers and the Worker config together. Add other hosting origins only after
approval. Localhost is not permitted by production CORS.

## Request and privacy rules

`POST /feedback` accepts JSON with `source`, `version`, `commit`, `kind`, `message`,
optional `email`, and `context`. The Turnstile token goes in `X-Turnstile-Token`, not
in the saved JSON. `context` can be `null`. Requests are capped at 16 KiB, messages at
4000 characters and version strings at 40 characters. Unknown fields and unrecognized
context shapes fail validation.

Only the GitHub Pages origin is permitted. Cloudflare's rate-limit binding allows
five attempts per IP per minute, including invalid requests. This is an approximate,
per-Cloudflare-location limit, not a global counter. Every accepted request also needs
a valid Turnstile response with the `feedback` action and the expected hostname.

The app keeps at most 20 registry command IDs and 10 diagnostic categories in memory
for this tab. It does not collect command arguments or raw log/error text, since those
can contain document data. The review shows the exact saved JSON, including build
identity. The context checkbox removes diagnostics, browser/OS, screen, session time
and document summary. Source, version, commit and the typed message remain visible.

Document size classes use item counts: empty is 0, small is 1 to 50, medium is 51 to
500 and large is above 500. Part items are features, drawing items are views plus
annotations, and assembly items are occurrences. No file contents, names or paths
are collected. People can still type sensitive data into their own message, so the
form asks them not to.

IP addresses are used only as transient rate-limit keys. They are not saved in D1 or
GitHub issues and are not passed to Turnstile verification. Worker observability is
disabled. Cloudflare and GitHub process the submitted feedback as service providers.
The spam-check script loads only after the person chooses Review feedback. Support
remains available at `support@zigrawbohnbawerick.org` when a check or submission fails.

## Check changes

```sh
node --test feedback/tests/*.test.mjs
python3 -m unittest discover -s feedback/tests -p 'test_*.py'
NODE_PATH=/path/to/playwright/node_modules node feedback/tests/form.browser.mjs
(cd site/app && bun run lint && bun run test && bun run build:pages)
cargo check --locked -p soliddraft-web --target wasm32-unknown-unknown
(cd crates/soliddraft-web && trunk build --locked --public-url ./)
```

For browser checks, open the site form from its nav link. Review and submit a test
message. In the app, use Give feedback, then open an invalid `.dwprt` file to check
the error offer. Check the preview and context opt-out before sending. Verify that a
failed submission keeps the message and that keyboard focus stays in the app dialog.
The DOM feedback controls remain usable after the Wasm app stops.

For deterministic delivery tests, use a separate temporary Worker and D1 database.
Mock Turnstile only in that test deployment, never in production. Verify both sources,
run sync against the test config twice, check that each creates one private issue,
label the issues `test` and close them. Delete the test Worker and database afterward.
Also check that the production Worker rejects missing and invalid Turnstile tokens.
A person must complete one live production verification after publishing the frontend;
a headless-browser rejection is not proof that a valid human challenge works.
