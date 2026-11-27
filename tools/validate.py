"""Validate this design/documentation bundle; never execute product/network operations."""
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit
import json
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
errors = []


class Page(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.ids = set()
        self.links = []
        self.lang = None
        self.title = False
        self.viewport = False

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            if attrs["id"] in self.ids:
                errors.append(f"Duplicate id: {attrs['id']}")
            self.ids.add(attrs["id"])
        if tag == "html":
            self.lang = attrs.get("lang")
        if tag == "title":
            self.title = True
        if tag == "meta" and attrs.get("name") == "viewport":
            self.viewport = True
        for key in ("href", "src"):
            if attrs.get(key):
                self.links.append(attrs[key])


pages = {}
for file in ROOT.rglob("*.html"):
    parser = Page()
    parser.feed(file.read_text(encoding="utf-8"))
    pages[file] = parser
    if parser.lang != "ru" or not parser.title or not parser.viewport:
        errors.append(f"Missing lang/title/viewport: {file.relative_to(ROOT)}")


def check_link(source, link):
    parts = urlsplit(link)
    if parts.scheme or parts.netloc:
        if parts.scheme not in ("https", "mailto"):
            errors.append(f"Unexpected URL scheme in {source}: {link}")
        return
    target = (source.parent / unquote(parts.path)).resolve() if parts.path else source
    if not target.exists():
        errors.append(f"Missing link in {source.relative_to(ROOT)}: {link}")
    elif parts.fragment and target in pages and parts.fragment not in pages[target].ids:
        errors.append(f"Missing anchor in {source.relative_to(ROOT)}: {link}")


for source, parser in pages.items():
    for link in parser.links:
        check_link(source, link)

for file in ROOT.rglob("*.md"):
    for link in re.findall(r"(?<!!)\[[^\]]+\]\(([^)\s]+)\)", file.read_text(encoding="utf-8")):
        check_link(file, link)

for file in ROOT.rglob("*.json"):
    json.loads(file.read_text(encoding="utf-8"))

script = "\n".join(file.read_text(encoding="utf-8") for file in (ROOT / "mockups").glob("*.js"))
if re.search(r"\b(fetch|XMLHttpRequest|WebSocket|sendBeacon)\s*\(", script):
    errors.append("Prototype must not perform network operations")
if "localStorage.setItem(\"orkestr-demo-theme\", theme)" not in script:
    errors.append("Expected theme-only local storage persistence")
if re.search(r"localStorage\.setItem\((?!\"orkestr-demo-theme\")", script):
    errors.append("Unexpected persistent prototype data")

for file in ROOT.rglob("*.css"):
    text = file.read_text(encoding="utf-8")
    if "http" in text or "@import" in text:
        errors.append(f"Unexpected remote stylesheet/import: {file.name}")
    for token in re.findall(r"var\((--[\w-]+)", text):
        if token not in (ROOT / "tokens.css").read_text(encoding="utf-8"):
            errors.append(f"Undefined token {token} in {file.name}")

if errors:
    print("FAILED")
    for error in errors:
        print("-", error)
    sys.exit(1)
print(f"PASS: {len(pages)} HTML pages; local HTML/Markdown links and anchors; JSON; CSS tokens; offline JS guard.")
print("This is not a browser-render, Rust build, security certification or network test.")
