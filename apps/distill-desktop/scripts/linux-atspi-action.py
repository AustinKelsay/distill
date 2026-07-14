#!/usr/bin/env python3
"""Activate a named interactive accessible through its AT-SPI action."""

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


def matches(candidate, expected, contains):
    return expected in candidate if contains else candidate == expected


def activate(expected, contains, deadline):
    while time.monotonic() < deadline:
        desktop = Atspi.get_desktop(0)
        for node in descendants(desktop):
            try:
                candidate = node.get_name() or ""
                if not matches(candidate, expected, contains):
                    continue
                component = node.get_component_iface()
                if component is not None:
                    try:
                        component.scroll_to(Atspi.ScrollType.ANYWHERE)
                        component.grab_focus()
                    except Exception:
                        pass
                action = node.get_action_iface()
                if action is None:
                    continue
                count = action.get_n_actions()
                for index in range(count):
                    name = (action.get_action_name(index) or "").lower()
                    if name in {"click", "press", "activate", "invoke"} or index == 0:
                        if action.do_action(index):
                            return {"name": candidate, "action": name or str(index)}
            except Exception:
                continue
        time.sleep(0.25)
    print(f"AT-SPI action not found or not activatable: {expected}", file=sys.stderr)
    sys.exit(1)


parser = argparse.ArgumentParser()
parser.add_argument("--name", required=True)
parser.add_argument("--contains", action="store_true")
parser.add_argument("--timeout", type=float, default=20.0)
args = parser.parse_args()

Atspi.init()
print(json.dumps(activate(args.name, args.contains, time.monotonic() + args.timeout)))
