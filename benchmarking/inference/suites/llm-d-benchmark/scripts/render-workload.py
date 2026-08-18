#!/usr/bin/env python3
"""Scale a bundled upstream workload without changing its traffic shape."""

import argparse
from decimal import Decimal
from pathlib import Path
import re


RATE = re.compile(r"^(\s*- rate:\s*)([0-9]+(?:\.[0-9]+)?)(\s*)$", re.MULTILINE)
TIMEOUT = re.compile(r"^(\s*request_timeout:\s*)[0-9]+(\s*)$", re.MULTILINE)


def decimal_text(value: Decimal) -> str:
    rendered = format(value.normalize(), "f")
    return rendered.rstrip("0").rstrip(".") if "." in rendered else rendered


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--rate-scale", required=True, type=Decimal)
    parser.add_argument("--request-timeout", required=True, type=int)
    args = parser.parse_args()

    if args.rate_scale <= 0:
        parser.error("--rate-scale must be greater than zero")
    if args.request_timeout <= 0:
        parser.error("--request-timeout must be greater than zero")

    source = args.input.read_text(encoding="utf-8")
    rendered, rate_count = RATE.subn(
        lambda match: (
            f"{match.group(1)}"
            f"{decimal_text(Decimal(match.group(2)) * args.rate_scale)}"
            f"{match.group(3)}"
        ),
        source,
    )
    rendered, timeout_count = TIMEOUT.subn(
        rf"\g<1>{args.request_timeout}\g<2>", rendered
    )
    if rate_count == 0 or timeout_count != 1:
        raise ValueError(
            f"unexpected workload structure: rates={rate_count}, timeouts={timeout_count}"
        )
    args.output.write_text(rendered, encoding="utf-8")


if __name__ == "__main__":
    main()
