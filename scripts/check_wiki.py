"""Check the wiki pages: every link between pages resolves, and every `ov` block compiles.

    python3 scripts/check_wiki.py target/release/ovc

Standard library only. Exits with 1 when a link or a block is broken.
"""
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
WIKI = ROOT / "wiki"
LINK = re.compile(r"\]\(([^)\s]+)\)")
BLOCK = re.compile(r"^```ov\n(.*?)^```", re.MULTILINE | re.DOTALL)


def check_links(pages):
    broken = []
    for page in pages:
        for target in LINK.findall(page.read_text(encoding="utf-8")):
            if target.startswith(("http://", "https://", "#")):
                continue
            name = target.split("#", 1)[0]
            if not (WIKI / f"{name}.md").exists():
                broken.append(f"{page.name}: link to '{target}' has no page")
    return broken


def check_blocks(pages, ovc):
    failed = []
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        for page in pages:
            for n, code in enumerate(BLOCK.findall(page.read_text(encoding="utf-8")), 1):
                src = tmp / f"{page.stem}_{n}.ov"
                src.write_text(code, encoding="utf-8")
                run = subprocess.run([ovc, str(src), "-o", str(tmp / f"{src.stem}.sv")],
                                     capture_output=True, text=True)
                if run.returncode != 0:
                    failed.append(f"{page.name}, ov block {n}:\n{run.stderr.strip()}")
                else:
                    print(f"ok   {page.name}, ov block {n}")
    return failed


def main():
    if len(sys.argv) != 2:
        sys.exit("usage: check_wiki.py <path to ovc>")
    ovc = str(pathlib.Path(sys.argv[1]).resolve())
    pages = sorted(WIKI.glob("*.md"))
    broken = check_links(pages) + check_blocks(pages, ovc)
    for line in broken:
        print(line, file=sys.stderr)
    if broken:
        sys.exit(1)
    print(f"{len(pages)} pages checked")


if __name__ == "__main__":
    main()
