#!/usr/bin/env python3
"""Check tracked Markdown/Org links and local GitHub targets without network I/O."""

from __future__ import annotations

import argparse
from collections import Counter
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import unquote, urlsplit


REPOSITORY = Path(__file__).resolve().parent.parent
INLINE_LINK = re.compile(r"!?\[[^\]\n]*\]\(\s*(<[^>\n]+>|[^\s)]+)(?:\s+[^)\n]*)?\)")
REFERENCE_LINK = re.compile(r"^\s{0,3}\[[^]\n]+\]:\s*(<[^>\n]+>|\S+)")
ORG_LINK = re.compile(r"\[\[([^]\n]+)\]")
HTML_ANCHOR = re.compile(r"\b(?:id|name)=[\"']([^\"']+)[\"']")


def tracked_documents(root: Path) -> list[Path]:
    result = subprocess.check_output(
        ["git", "ls-files", "-z", "--", "*.md", "*.org"], cwd=root
    )
    return [root / name.decode("utf-8") for name in result.split(b"\0") if name]


def prose_lines(document: Path) -> list[tuple[int, str]]:
    """Ignore literal examples; retain line numbers for actionable diagnostics."""
    result = []
    fence = None
    org_block = False
    for number, line in enumerate(document.read_text(encoding="utf-8").splitlines(), 1):
        if document.suffix == ".org":
            if re.match(r"\s*#\+begin_(src|example|export)\b", line, re.I):
                org_block = True
            if org_block:
                if re.match(r"\s*#\+end_(src|example|export)\b", line, re.I):
                    org_block = False
                continue
        else:
            match = re.match(r"^\s{0,3}(`{3,}|~{3,})", line)
            if match:
                marker = match[1]
                if fence is None:
                    fence = marker
                elif marker[0] == fence[0] and len(marker) >= len(fence):
                    fence = None
                continue
            if fence:
                continue
        result.append((number, line))
    return result


def heading_slug(text: str) -> str:
    text = re.sub(r"\[([^]]+)\]\([^)]*\)", r"\1", text)
    text = re.sub(r"<[^>]+>", "", text)
    return re.sub(r"[^\w\- ]", "", text.lower()).replace(" ", "-")


def anchors(document: Path) -> tuple[set[str], set[str]]:
    ids: set[str] = set()
    titles: set[str] = set()
    counts: Counter[str] = Counter()
    lines = prose_lines(document)
    for index, (_, line) in enumerate(lines):
        ids.update(HTML_ANCHOR.findall(line))
        custom = re.match(r"\s*:(?:CUSTOM_ID|ID):\s+(\S+)", line, re.I)
        if custom:
            ids.add(custom[1])
        pattern = r"^\*+\s+(.+)" if document.suffix == ".org" else r"^ {0,3}#{1,6}\s+(.+?)\s*#*\s*$"
        match = re.match(pattern, line)
        title = match[1] if match else None
        if document.suffix == ".md" and index and re.fullmatch(r" {0,3}(?:=+|-+)\s*", line):
            title = lines[index - 1][1].strip()
        if title:
            titles.add(title)
            slug = heading_slug(title)
            count = counts[slug]
            candidate = f"{slug}-{count}" if count else slug
            while candidate in ids:
                count += 1
                candidate = f"{slug}-{count}"
            counts[slug] = count + 1
            ids.add(candidate)
    return ids, titles


def links(document: Path) -> list[tuple[int, str]]:
    result = []
    for number, line in prose_lines(document):
        if document.suffix == ".org":
            result.extend((number, m[1]) for m in ORG_LINK.finditer(line))
        else:
            # Inline code can contain example Markdown that is not a real link.
            line = re.sub(r"(`+).*?\1", "", line)
            result.extend((number, m[1].strip("<>")) for m in INLINE_LINK.finditer(line))
            reference = REFERENCE_LINK.match(line)
            if reference:
                result.append((number, reference[1].strip("<>")))
    return result


def github_target(target: str, repositories: dict[str, Path]) -> tuple[Path, str, str] | None:
    parsed = urlsplit(target)
    if parsed.scheme not in ("https", "http") or parsed.netloc.lower() != "github.com":
        return None
    parts = parsed.path.strip("/").split("/")
    if len(parts) < 5 or parts[0].lower() != "bartbunting":
        return None
    repository, kind, revision = parts[1:4]
    if repository not in repositories or kind not in ("blob", "tree") or revision not in ("main", "master"):
        return None  # Commit/tag-pinned evidence describes that historical tree.
    return repositories[repository], unquote("/".join(parts[4:])), unquote(parsed.fragment)


def check_document(
    document: Path,
    root: Path,
    repositories: dict[str, Path],
    *,
    incoming_only: bool = False,
) -> list[str]:
    errors = []
    if not document.is_file():
        return [f"{document}: tracked document is missing"]
    for number, target in links(document):
        github = github_target(target, repositories)
        heading = None
        if incoming_only:
            if github is None or github[0] != repositories["omnivox"]:
                continue
        if github:
            owner, path, fragment = github
            resolved = owner / path
        else:
            if urlsplit(target).scheme in ("https", "http", "mailto"):
                continue
            value = target.removeprefix("file:")
            if document.suffix == ".org" and not target.startswith(("file:", "#")):
                # Org fuzzy links address headings/targets in the current document.
                owner, resolved, fragment = root, document, ""
                heading = value.lstrip("*")
            else:
                value, separator, search = value.partition("::")
                parsed = urlsplit(value)
                if parsed.scheme or parsed.netloc:
                    errors.append(f"{document}:{number}: unsupported link {target!r}")
                    continue
                owner = root
                resolved = document.parent / unquote(parsed.path) if parsed.path else document
                fragment = unquote(parsed.fragment)
                if separator:
                    if search.startswith("#"):
                        fragment = search[1:]
                    elif search.startswith("*"):
                        heading = search.lstrip("*")
                    elif search.isdigit():
                        fragment = "L" + search
                    else:
                        errors.append(f"{document}:{number}: unsupported Org search {target!r}")
                        continue
        resolved = resolved.resolve()
        if not resolved.is_relative_to(owner.resolve()):
            errors.append(f"{document}:{number}: link leaves repository: {target}")
        elif not resolved.exists():
            errors.append(f"{document}:{number}: missing target {target}")
        elif fragment or heading:
            if resolved.is_dir():
                resolved = resolved / "README.md"
            line_ref = re.fullmatch(r"L([1-9][0-9]*)(?:-L([1-9][0-9]*))?", fragment)
            if line_ref and resolved.is_file():
                first = int(line_ref[1]); last = int(line_ref[2] or first)
                valid = first <= last <= len(resolved.read_text(encoding="utf-8").splitlines())
            elif resolved.is_file() and resolved.suffix in (".md", ".org"):
                ids, titles = anchors(resolved)
                valid = heading in titles or heading in ids if heading else fragment in ids
            else:
                valid = False
            if not valid:
                errors.append(f"{document}:{number}: missing anchor/search in {target}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--emacsvox-source", type=Path, help="also check incoming Emacsvox links against this checkout")
    args = parser.parse_args()
    repositories = {"omnivox": REPOSITORY}
    if args.emacsvox_source:
        repositories["emacsvox"] = args.emacsvox_source.resolve()
    documents = tracked_documents(REPOSITORY)
    errors = []
    for document in documents:
        errors.extend(check_document(document, REPOSITORY, repositories))
    paired = 0
    if args.emacsvox_source:
        peer = repositories["emacsvox"]
        for document in tracked_documents(peer):
            paired += 1
            errors.extend(check_document(document, peer, repositories, incoming_only=True))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Checked targets and anchors in {len(documents)} Markdown/Org documents")
    if paired:
        print(f"Checked incoming Omnivox links in {paired} Emacsvox documents")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
