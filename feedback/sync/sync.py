#!/usr/bin/env python3
"""Copy private D1 feedback into private GitHub issues using local CLI logins."""

import argparse
import base64
import fcntl
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

REPO = "ZIGRAWBOHNBAWERICK/soliddraft-feedback"
CONFIG = Path(__file__).resolve().parents[1] / "worker" / "wrangler.toml"


def run(argv, body=None):
    result = subprocess.run(argv, input=body, text=True, capture_output=True, timeout=120)
    if result.returncode:
        # CLI stderr can contain feedback or credentials. Do not copy it to logs.
        raise RuntimeError(f"{argv[0]} failed (exit {result.returncode}); check its login and service availability")
    return result.stdout.strip()


def github(path, method="GET", body=None):
    args = ["gh-axi", "api", method, f"/repos/{REPO}{path}", "--jq", "@base64", "--full"]
    if body is not None:
        args += ["--input", "-"]
    return json.loads(base64.b64decode(run(args, None if body is None else json.dumps(body))))


def d1(sql, config=CONFIG):
    raw = run(["npx", "--yes", "wrangler@4.158.0", "d1", "execute", "DB",
               "--remote", "--cwd", str(config.parent), "--config", str(config), "--command", sql, "--json"])
    result = json.loads(raw)
    if not result or not all(r.get("success") for r in result):
        raise RuntimeError("D1 query failed")
    return result[0]["results"]


def quote(value):
    return "'" + value.replace("'", "''") + "'"


def marker(row_id):
    return f"<!-- soliddraft-feedback:{row_id} -->"


def fenced(value, language=""):
    fence = "`" * max(3, 1 + max((len(s) for s in re.findall(r"`+", value)), default=0))
    return f"{fence}{language}\n{value}\n{fence}"


def issue_body(row, payload):
    return "\n\n".join([
        marker(row["id"]),
        f"Source: {payload['source']}\nVersion: {payload['version']}\nCommit: {payload['commit']}\nReceived: {row['created_at']}",
        "Message:\n" + fenced(payload["message"]),
        "Contact:\n" + fenced(payload.get("email", "Not supplied")),
        "<details><summary>Diagnostic context</summary>\n\n" +
        fenced(json.dumps(payload["context"], indent=2), "json") + "\n\n</details>",
    ])


def sync(config=CONFIG):
    if github("").get("private") is not True:
        raise RuntimeError("Refusing to sync into a public repository")
    pending = d1("SELECT id, created_at, payload FROM feedback WHERE issue_url IS NULL ORDER BY created_at LIMIT 100", config)
    if not pending:
        print("No pending feedback.")
        return
    existing = {}
    labels = set()
    page = 1
    while True:
        issues = github(f"/issues?state=all&per_page=100&page={page}")
        for issue in issues:
            for row_id in re.findall(r"\A<!-- soliddraft-feedback:([a-f0-9-]{36}) -->", issue.get("body") or ""):
                existing[row_id] = issue["html_url"]
        if len(issues) < 100:
            break
        page += 1
    page = 1
    while True:
        batch = github(f"/labels?per_page=100&page={page}")
        labels.update(label["name"] for label in batch)
        if len(batch) < 100:
            break
        page += 1
    for row in pending:
        payload = json.loads(row["payload"])
        url = existing.get(row["id"])
        if not url:
            required = [f"source:{payload['source']}", f"kind:{payload['kind']}", f"version:{payload['version']}"]
            for label in required:
                if label not in labels:
                    github("/labels", "POST", {"name": label, "color": "64748b"})
                    labels.add(label)
            issue = github("/issues", "POST", {
                "title": f"[{payload['source']}] {payload['kind']} feedback ({row['id'][:8]})",
                "body": issue_body(row, payload), "labels": required,
            })
            url = issue["html_url"]
        d1(f"UPDATE feedback SET issue_url={quote(url)}, synced_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id={quote(row['id'])} AND issue_url IS NULL", config)
        print(f"Synced {row['id']} to {url}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=CONFIG, help="Worker config whose DB binding to sync")
    parser.add_argument("--lock", type=Path, default=Path(tempfile.gettempdir()) / "soliddraft-feedback-sync.lock")
    args = parser.parse_args()
    with args.lock.open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print("Another sync is running.")
            return
        sync(args.config.resolve())


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.TimeoutExpired, json.JSONDecodeError, KeyError, OSError):
        print("Feedback sync failed. No pending rows were deleted; check CLI logins and retry.", file=sys.stderr)
        sys.exit(1)
