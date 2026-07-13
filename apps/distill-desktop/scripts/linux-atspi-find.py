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


def find_named(name, contains, deadline):
    while time.monotonic() < deadline:
        desktop = Atspi.get_desktop(0)
        for node in descendants(desktop):
            try:
                candidate_name = node.get_name() or ""
                matches = name in candidate_name if contains else candidate_name == name
                if not matches:
                    continue
                return {
                    "name": candidate_name,
                    "role": node.get_role_name(),
                }
            except Exception:
                continue
        time.sleep(0.25)
    return None


parser = argparse.ArgumentParser()
parser.add_argument("--name", required=True)
parser.add_argument("--contains", action="store_true")
parser.add_argument("--timeout", type=float, default=20.0)
args = parser.parse_args()

found = find_named(args.name, args.contains, time.monotonic() + args.timeout)
if found is None:
    print(f"AT-SPI accessible not found: {args.name}", file=sys.stderr)
    sys.exit(1)
print(json.dumps(found))
