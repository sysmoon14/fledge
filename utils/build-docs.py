#!/usr/bin/env python3
"""Builds the documentation pages: each site/docs/pages/<slug>.html body is
wrapped in the docs' header, sidebar and footer, with links to the pages
before and after it, into DIR/docs (default dist/site/docs).

    python3 utils/build-docs.py [DIR]

A page starts with two comment lines, then its body:

    <!-- title: The glance -->
    <!-- description: What the Nest shows from across the room. -->
    <h1>The glance</h1>
    ...

Add a page to NAV below to put it in the sidebar; pages not in NAV are an
error, so nothing is published unlinked. Standard library only.
"""
import html
import re
import shutil
import sys
from pathlib import Path

NAV = [
    ("Getting started", [
        ("index", "Overview"),
        ("install", "Install on the thermostat"),
        ("integration", "Add it to Home Assistant"),
    ]),
    ("On the thermostat", [
        ("cards", "Cards and the ring"),
        ("thermostat", "The thermostat and Settings"),
        ("glance", "The glance"),
        ("alarm", "Alarm panels"),
    ]),
    ("In Home Assistant", [
        ("panel", "The Fledge panel"),
        ("automations", "Wake and automations"),
        ("notifications", "Notifications"),
        ("schedules", "Schedules"),
        ("weather", "Weather and room temperature"),
    ]),
    ("Looking after it", [
        ("updates", "Updates"),
        ("undo", "Back up and undo"),
        ("troubleshooting", "Troubleshooting"),
        ("security", "Security"),
    ]),
    ("Reference", [
        ("reference", "Entities, actions, CLI"),
        ("testing", "In testing: v0.10"),
    ]),
]

HEAD = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} · Fledge docs</title>
<meta name="description" content="{description}">
<link rel="icon" href="../fledge.svg" type="image/svg+xml">
<link rel="stylesheet" href="docs.css">
</head>
<body>
<header class="top">
  <a class="brand" href="../"><img src="../fledge.svg" alt="">Fledge <span>docs</span></a>
  <nav aria-label="Site">
    <a class="wide" href="../">Home</a>
    <a class="wide" href="https://github.com/sysmoon14/fledge/releases">Releases</a>
    <a href="https://github.com/sysmoon14/fledge">GitHub</a>
  </nav>
</header>
<div class="layout">
<aside class="side">
<button class="menu-toggle" type="button" aria-expanded="false" aria-controls="docs-nav">Contents: {title}</button>
<nav id="docs-nav" aria-label="Documentation" data-open="false">
{sidebar}
</nav>
</aside>
<main>
"""

FOOT = """{pager}
</main>
</div>
<footer>
  Fledge is a fork of <a href="https://github.com/unfr0zen/unfr0zen">unfr0zen</a>, licensed under the GNU GPL v3.0.
  It is not affiliated with, endorsed or sponsored by Google LLC or Nest Labs; Nest and Nest Learning Thermostat are trademarks of Google LLC.
  Something wrong or missing here? <a href="https://github.com/sysmoon14/fledge/issues/new">Open an issue</a>.
</footer>
<script>
// The contents button on narrow screens; headings get a link to themselves.
(() => {
  const b = document.querySelector(".menu-toggle"), n = document.getElementById("docs-nav");
  b.addEventListener("click", () => {
    const open = n.dataset.open !== "true";
    n.dataset.open = open; b.setAttribute("aria-expanded", open);
  });
  for (const h of document.querySelectorAll("main h2[id], main h3[id]")) {
    const a = document.createElement("a");
    a.href = "#" + h.id; a.className = "anchor"; a.textContent = "#";
    a.setAttribute("aria-label", "Link to this section");
    h.append(a);
  }
})();
</script>
</body>
</html>
"""


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    src = root / "site" / "docs"
    out = Path(sys.argv[1] if len(sys.argv) > 1 else root / "dist" / "site") / "docs"
    order = [slug for _, pages in NAV for slug, _ in pages]
    names = {slug: name for _, pages in NAV for slug, name in pages}

    found = {p.stem for p in (src / "pages").glob("*.html")}
    if found != set(order):
        sys.exit(f"pages and NAV differ: missing {sorted(set(order) - found)}, unlisted {sorted(found - set(order))}")

    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    shutil.copy(src / "docs.css", out / "docs.css")

    for i, slug in enumerate(order):
        text = (src / "pages" / f"{slug}.html").read_text()
        meta = dict(re.findall(r"<!--\s*(title|description):\s*(.*?)\s*-->", text))
        body = re.sub(r"\A(\s*<!--.*?-->\s*)+", "", text, flags=re.S)
        sidebar = []
        for group, pages in NAV:
            sidebar.append(f"<p>{html.escape(group)}</p>\n<ul>")
            for s, name in pages:
                current = ' aria-current="page"' if s == slug else ""
                sidebar.append(f'  <li><a href="{s}.html"{current}>{html.escape(name)}</a></li>')
            sidebar.append("</ul>")
        pager = ['<nav class="pager" aria-label="Next and previous">']
        if i > 0:
            p = order[i - 1]
            pager.append(f'  <a href="{p}.html"><span>Previous</span>{html.escape(names[p])}</a>')
        if i + 1 < len(order):
            n = order[i + 1]
            pager.append(f'  <a class="next" href="{n}.html"><span>Next</span>{html.escape(names[n])}</a>')
        pager.append("</nav>")
        page = HEAD.format(
            title=html.escape(meta.get("title", names[slug])),
            description=html.escape(meta.get("description", ""), quote=True),
            sidebar="\n".join(sidebar),
        ) + body.strip() + "\n" + FOOT.replace("{pager}", "\n".join(pager))
        (out / f"{slug}.html").write_text(page)
    print(f"{len(order)} documentation pages in {out}")


if __name__ == "__main__":
    main()
