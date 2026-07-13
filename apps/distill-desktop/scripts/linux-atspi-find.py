#!/usr/bin/env python3
"""Find any Distill accessible by name for packaged Linux smoke waits/assertions.

Unlike linux-atspi-bounds.py, this helper accepts non-interactive roles so status
text such as "Status: warning" can be observed. It is not screen-reader evidence.
"""

import argparse
import json
import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

Atspi.init()


def descendants(node):
    yield node
    try:
        count = node.get_child_count()
    except Exception:
        return
    for index in range(count):
        try:
            child = node.get_child_at_index(index)
        except Exception:
            continue
        if child is not None:
            yield from descendants(child)


def candidate_values(node):
    """Return the accessible name and any exposed AT-SPI text content."""
    values = []
    try:
        values.append(node.get_name() or "")
    except Exception:
        pass
    try:
        text = node.get_text_iface()
        if text is not None:
            count = text.get_character_count()
            if count > 0:
                values.append(text.get_text(0, count) or "")
    except Exception:
        pass
    return tuple(value for value in values if value)


def find_named(name, contains, deadline):
    observed_statuses = set()
    while time.monotonic() < deadline:
        desktop = Atspi.get_desktop(0)
        for node in descendants(desktop):
            try:
                for candidate in candidate_values(node):
                    if candidate.startswith("Status:") or any(
                        candidate.startswith(f"{kind}:")
                        for kind in ("fixture", "codex", "claude_code", "opencode", "droid")
                    ):
                        observed_statuses.add(candidate)
                    matches = name in candidate if contains else candidate == name
                    if matches:
                        return {"name": candidate, "role": node.get_role_name()}, observed_statuses
            except Exception:
                continue
        time.sleep(0.25)
    return None, observed_statuses


parser = argparse.ArgumentParser()
parser.add_argument("--name", required=True)
parser.add_argument("--contains", action="store_true")
parser.add_argument("--timeout", type=float, default=20.0)
args = parser.parse_args()

found, observed_statuses = find_named(args.name, args.contains, time.monotonic() + args.timeout)
if found is None:
    suffix = "; observed: " + ", ".join(sorted(observed_statuses)) if observed_statuses else ""
    print(f"AT-SPI accessible not found: {args.name}{suffix}", file=sys.stderr)
    sys.exit(1)
print(json.dumps(found))
