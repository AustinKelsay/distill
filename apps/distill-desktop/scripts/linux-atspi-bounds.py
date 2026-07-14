#!/usr/bin/env python3
"""Find a visible Distill WebKit control by its AT-SPI accessible name."""

import argparse
import json
import sys
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi


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


INTERACTIVE_ROLES = {
    "check box",
    "combo box",
    "entry",
    "editable text",
    "push button",
    "radio button",
    "text",
    "toggle button",
}


def find_control(name, contains, interactive, deadline):
    while time.monotonic() < deadline:
        desktop = Atspi.get_desktop(0)
        for application in descendants(desktop):
            try:
                candidate_name = application.get_name() or ""
                matches_name = name in candidate_name if contains else candidate_name == name
                if not matches_name:
                    continue
                role = application.get_role_name()
                if interactive and role not in INTERACTIVE_ROLES:
                    continue
                component = application.get_component_iface()
                if component is None:
                    continue
                try:
                    component.scroll_to(Atspi.ScrollType.ANYWHERE)
                except Exception:
                    pass
                bounds = component.get_extents(Atspi.CoordType.SCREEN)
                if bounds.width > 0 and bounds.height > 0:
                    return {
                        "name": candidate_name,
                        "role": role,
                        "x": bounds.x,
                        "y": bounds.y,
                        "width": bounds.width,
                        "height": bounds.height,
                    }
            except Exception:
                continue
        time.sleep(0.25)
    return None


parser = argparse.ArgumentParser()
parser.add_argument("--name", required=True)
parser.add_argument("--contains", action="store_true")
parser.add_argument("--interactive", action="store_true")
parser.add_argument("--timeout", type=float, default=20.0)
args = parser.parse_args()

Atspi.init()

control = find_control(args.name, args.contains, args.interactive, time.monotonic() + args.timeout)
if control is None:
    print(f"AT-SPI control not found: {args.name}", file=sys.stderr)
    sys.exit(1)

print(json.dumps(control))
