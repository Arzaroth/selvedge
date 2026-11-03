#!/usr/bin/env python3
"""Compares each pricing page's dollar figures with the ones src/catalog.json
last saw there, and prints what moved.

A page whose HTML carries no prices (`rendered: client`) cannot be checked
this way and is listed as such. A page that refuses the fetch is reported as
unread, never as changed. Exits 1 when a figure moved, 0 otherwise.
"""

import html
import json
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

CATALOG = Path(__file__).resolve().parent.parent / "src" / "catalog.json"
USER_AGENT = "selvedge-catalog-canary (+https://github.com/Arzaroth/selvedge)"


def figures(page):
    text = re.sub(r"(?s)<(script|style)[^>]*>.*?</\1>", " ", page)
    text = html.unescape(re.sub(r"<[^>]+>", " ", text))
    found = set()
    for m in re.finditer(r"\$\s?(\d+(?:,\d{3})*(?:\.\d+)?)", text):
        found.add(float(m.group(1).replace(",", "")))
    return found


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read().decode("utf-8", "replace")


def check(provider, pricing, read=fetch):
    url = pricing["url"]
    if pricing["rendered"] != "server":
        return ("unchecked", f"{provider}: {url} fills its prices in with a script")
    try:
        seen = figures(read(url))
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        return ("unread", f"{provider}: {url} could not be read ({e})")
    expected = set(float(f) for f in pricing.get("figures", []))
    if not seen:
        return ("unread", f"{provider}: {url} showed no dollar figure at all")
    added, gone = sorted(seen - expected), sorted(expected - seen)
    if not added and not gone:
        return ("same", f"{provider}: {url} unchanged")
    def show(xs):
        return ", ".join(f"${x:g}" for x in xs) or "none"
    return ("changed", f"{provider}: {url} now shows {show(added)} new, {show(gone)} gone")


def main():
    catalog = json.loads(CATALOG.read_text())
    results = [check(name, p["pricing"]) for name, p in catalog["providers"].items()]
    for kind in ("changed", "unread", "unchecked", "same"):
        for k, line in results:
            if k == kind:
                print(f"{kind}: {line}")
    return 1 if any(k == "changed" for k, _ in results) else 0


if __name__ == "__main__":
    sys.exit(main())
