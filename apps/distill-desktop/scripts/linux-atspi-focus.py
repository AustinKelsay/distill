#!/usr/bin/env python3
"""AT-SPI focus probes for Distill Linux packaged smoke.

Locates named accessibles and reports FOCUSED state under Ubuntu/Xvfb.
This is install/runtime smoke evidence only — not screen-reader conformance.
"""

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


def node_name(node):
    try:
        return node.get_name() or ""
    except Exception:
        return ""


def node_role(node):
    try:
        return node.get_role_name() or ""
    except Exception:
        return ""


def matches_name(candidate_name, name, contains):
    if contains:
        return name in candidate_name
    return candidate_name == name


def is_focused(node):
    try:
        state_set = node.get_state_set()
    except Exception:
        return False
    try:
        return state_set.contains(Atspi.StateType.FOCUSED)
    except Exception:
        return False


def serialize(node):
    return {"name": node_name(node), "role": node_role(node)}


def fail(message):
    print(message, file=sys.stderr)
    sys.exit(1)


def wait_assert_focused(name, contains, deadline):
    """Poll until a named accessible is focused, or fail with a typed message."""
    saw_control = False
    while time.monotonic() < deadline:
        desktop = Atspi.get_desktop(0)
        for node in descendants(desktop):
            candidate = node_name(node)
            if not matches_name(candidate, name, contains):
                continue
            saw_control = True
            if is_focused(node):
                return serialize(node)
        time.sleep(0.25)
    if not saw_control:
        fail(f"AT-SPI control not found: {name}")
    fail(f"AT-SPI control is not focused: {name}")


def focused_descendants(dialog):
    focused = []
    for node in descendants(dialog):
        if is_focused(node):
            focused.append(serialize(node))
    return focused


def wait_dialog_focus(name, contains, deadline):
    """Poll until a named dialog has a focused descendant, then report them."""
    saw_dialog = False
    while time.monotonic() < deadline:
        desktop = Atspi.get_desktop(0)
        for node in descendants(desktop):
            candidate = node_name(node)
            if not matches_name(candidate, name, contains):
                continue
            if node_role(node) != "dialog":
                continue
            saw_dialog = True
            focused = focused_descendants(node)
            if focused:
                return {"dialog": serialize(node), "focused": focused}
        time.sleep(0.25)
    if not saw_dialog:
        fail(f"AT-SPI dialog not found: {name}")
    fail(f"AT-SPI dialog has no focused descendant: {name}")


parser = argparse.ArgumentParser(
    description="AT-SPI focus probes for Distill Linux packaged smoke (not SR conformance)."
)
mode = parser.add_mutually_exclusive_group(required=True)
mode.add_argument(
    "--assert-focused",
    action="store_true",
    help="Find a named accessible and assert it is focused",
)
mode.add_argument(
    "--dialog-focus",
    action="store_true",
    help="Find a named dialog and report its focused descendants",
)
parser.add_argument("--name", required=True)
parser.add_argument("--contains", action="store_true")
parser.add_argument("--timeout", type=float, default=20.0)
args = parser.parse_args()

Atspi.init()
deadline = time.monotonic() + args.timeout

if args.assert_focused:
    result = wait_assert_focused(args.name, args.contains, deadline)
elif args.dialog_focus:
    result = wait_dialog_focus(args.name, args.contains, deadline)
else:
    fail("AT-SPI focus helper requires --assert-focused or --dialog-focus")

print(json.dumps(result))
