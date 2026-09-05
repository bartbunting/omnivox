#!/usr/bin/env python3
"""Create a private remote-service token without printing or replacing it."""

import argparse
import os
import secrets
from pathlib import Path


def create_token(path: Path) -> None:
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="ascii") as output:
        output.write(secrets.token_hex(32) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    args = parser.parse_args()
    try:
        create_token(args.path)
    except OSError as error:
        parser.exit(1, f"Could not create token file: {error}\n")
    print(f"Created private token file: {args.path}")


if __name__ == "__main__":
    main()
