#!/usr/bin/env python3
"""Verify that local links in tracked Markdown files resolve."""

from __future__ import annotations

from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import unquote, urlsplit


REPOSITORY = Path(__file__).resolve().parent.parent
MARKDOWN_LINK = re.compile(r"!?\[[^]]*\]\((?P<target>[^)\n]+)\)")
EXTERNAL_SCHEMES = {"http", "https", "mailto"}


def tracked_markdown_files() -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z", "--", "*.md"],
        cwd=REPOSITORY,
        check=True,
        stdout=subprocess.PIPE,
    )
    return [
        REPOSITORY / Path(name.decode("utf-8"))
        for name in result.stdout.split(b"\0")
        if name
    ]


def link_target(raw_target: str) -> str:
    target = raw_target.strip()
    if target.startswith("<"):
        closing = target.find(">")
        return target[1:closing] if closing >= 0 else target
    return target.split(maxsplit=1)[0]


def main() -> int:
    errors: list[str] = []
    documents = tracked_markdown_files()

    for document in documents:
        if not document.is_file():
            errors.append(f"{document.relative_to(REPOSITORY)}: tracked file is missing")
            continue
        text = document.read_text(encoding="utf-8")
        for match in MARKDOWN_LINK.finditer(text):
            target = link_target(match.group("target"))
            if target.startswith("#"):
                continue
            parsed = urlsplit(target)
            if parsed.scheme.lower() in EXTERNAL_SCHEMES:
                continue
            if parsed.scheme or parsed.netloc:
                errors.append(
                    f"{document.relative_to(REPOSITORY)}: unsupported link {target!r}"
                )
                continue
            path = unquote(parsed.path)
            if not path:
                continue
            resolved = (document.parent / path).resolve()
            try:
                resolved.relative_to(REPOSITORY)
            except ValueError:
                errors.append(
                    f"{document.relative_to(REPOSITORY)}: link leaves repository: {target}"
                )
                continue
            if not resolved.exists():
                errors.append(
                    f"{document.relative_to(REPOSITORY)}: missing local target {target}"
                )

    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Checked local links in {len(documents)} Markdown files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
