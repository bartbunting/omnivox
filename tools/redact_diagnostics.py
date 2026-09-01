#!/usr/bin/env python3
"""Remove private speech text and user-home paths from diagnostic text."""

from __future__ import annotations

import argparse
import re
import sys


SPEECH_TEXT_FIELDS = ("Captured synthesis text", "synthesis_text=")
GENERIC_HOME_PATTERNS = (
    re.compile(r"(?<![\w])/(?:home|Users)/[^/\s\"']+"),
    re.compile(r"(?i)\b[A-Z]:[\\/]+Users[\\/]+[^\\/\s\"']+"),
    re.compile(
        r"(?i)\\\\wsl(?:\.localhost)?\\[^\\\s\"']+"
        r"\\(?:home|Users)\\[^\\\s\"']+"
    ),
)


def redact(text: str, private_values: list[str]) -> str:
    retained = [
        line
        for line in text.splitlines(keepends=True)
        if not any(field in line for field in SPEECH_TEXT_FIELDS)
    ]
    redacted = "".join(retained)
    safe_values = sorted(
        {
            value.rstrip("/\\")
            for value in private_values
            if value and value.rstrip("/\\") not in ("", ".")
        },
        key=len,
        reverse=True,
    )
    for value in safe_values:
        redacted = redacted.replace(value, "<PRIVATE_PATH>")
    for pattern in GENERIC_HOME_PATTERNS:
        redacted = pattern.sub("<USER_HOME>", redacted)
    return redacted


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--private", action="append", default=[])
    return parser.parse_args()


def main() -> int:
    arguments = parse_arguments()
    sys.stdout.write(redact(sys.stdin.read(), arguments.private))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
