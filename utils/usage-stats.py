#!/usr/bin/env python3
"""Estimates how many people use Fledge from GitHub's own download counts,
with nothing sent from anyone's thermostat or Home Assistant.

    python3 utils/usage-stats.py [--repo OWNER/NAME] [--releases FILE]

Two counts per release:

- app: downloads of the Fledge app (Fledge-<platform>-<tag>.*). Someone
  setting up a thermostat, or trying to.
- updates: downloads of nestlocal-armv7. A thermostat installing that release
  from Home Assistant's update entity fetches it once (a failed or retried
  install fetches it again). The latest releases' counts are the closest to
  "thermostats running Fledge and keeping up to date".

Neither is a count of people: a download is not an install, one person may
have several thermostats, and a thermostat that never updates is not seen.
Read them as a trend from release to release. GITHUB_TOKEN, if set, raises
GitHub's rate limit. --releases reads a saved /releases response instead of
asking GitHub. Standard library only.
"""
import argparse
import json
import os
import sys
import urllib.request

UPDATE_ASSET = "nestlocal-armv7"
APP_PREFIX = "Fledge-"


def fetch(repo):
    releases, page = [], 1
    while True:
        req = urllib.request.Request(
            f"https://api.github.com/repos/{repo}/releases?per_page=100&page={page}",
            headers={"Accept": "application/vnd.github+json"})
        token = os.environ.get("GITHUB_TOKEN")
        if token:
            req.add_header("Authorization", f"Bearer {token}")
        with urllib.request.urlopen(req, timeout=30) as r:
            batch = json.load(r)
        releases += batch
        if len(batch) < 100:
            return releases
        page += 1


def platform(name):
    """Fledge-macos-arm64-v0.21.0.zip -> macos-arm64."""
    stem = name[len(APP_PREFIX):]
    return stem.rsplit("-v", 1)[0] if "-v" in stem else stem.split(".", 1)[0]


def summarise(releases):
    """One row per release, newest first: tag, date, pre-release, app and
    update downloads, and app downloads by platform."""
    rows = []
    for rel in releases:
        if rel.get("draft"):
            continue
        app, updates, by_platform = 0, 0, {}
        for asset in rel.get("assets", []):
            name, n = asset.get("name", ""), int(asset.get("download_count", 0))
            if name == UPDATE_ASSET:
                updates += n
            elif name.startswith(APP_PREFIX):
                app += n
                key = platform(name)
                by_platform[key] = by_platform.get(key, 0) + n
        rows.append({"tag": rel.get("tag_name", "?"), "date": (rel.get("published_at") or "")[:10],
                     "pre": bool(rel.get("prerelease")), "app": app, "updates": updates,
                     "platforms": by_platform})
    rows.sort(key=lambda r: r["date"], reverse=True)
    return rows


def report(rows):
    lines = [f"{'release':<16} {'published':<10} {'app':>6} {'updates':>8}"]
    for r in rows:
        tag = r["tag"] + (" (pre)" if r["pre"] else "")
        lines.append(f"{tag:<16} {r['date']:<10} {r['app']:>6} {r['updates']:>8}")
    totals = {}
    for r in rows:
        for k, n in r["platforms"].items():
            totals[k] = totals.get(k, 0) + n
    lines.append("")
    lines.append(f"app downloads, all releases: {sum(r['app'] for r in rows)}")
    for k, n in sorted(totals.items(), key=lambda kv: -kv[1]):
        lines.append(f"  {k}: {n}")
    latest = next((r for r in rows if not r["pre"]), None)
    if latest:
        lines.append(f"thermostats that installed {latest['tag']} from Home Assistant: about {latest['updates']}")
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--repo", default="sysmoon14/fledge")
    ap.add_argument("--releases", help="a saved /releases JSON response")
    ap.add_argument("--json", action="store_true", help="print the rows as JSON")
    args = ap.parse_args(argv)
    if args.releases:
        with open(args.releases) as f:
            releases = json.load(f)
    else:
        releases = fetch(args.repo)
    rows = summarise(releases)
    print(json.dumps(rows, indent=2) if args.json else report(rows))
    return 0


if __name__ == "__main__":
    sys.exit(main())
