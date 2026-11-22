#!/usr/bin/env python3
"""Compare a saved export with the export of its earlier accepted revision."""

import pathlib
import sys


def main():
    before, after = map(pathlib.Path, sys.argv[1:])
    expected = before.read_bytes()
    if not expected or after.read_bytes() != expected:
        raise SystemExit(f"{after} differs from the earlier export {before}")


if __name__ == "__main__":
    main()
