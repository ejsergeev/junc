#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
# From the validation (bench/validation/proto/a11y), unchanged otherwise.
"""Dump the AT-SPI tree of an application, or the currently focused object.

usage: atspi_dump.py tree  <app-name-substring>
       atspi_dump.py focus <app-name-substring>
Output: one line per accessible: depth, role, name, states, value (if Value iface), actions.
"""
import sys
import gi, warnings
warnings.filterwarnings("ignore")
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

INTERESTING = ["focusable", "focused", "checkable", "checked", "pressed", "indeterminate", "enabled", "sensitive",
               "selectable", "selected", "editable", "expandable", "expanded", "showing", "visible"]


def find_app(sub):
    desk = Atspi.get_desktop(0)
    for i in range(desk.get_child_count()):
        app = desk.get_child_at_index(i)
        if app and sub.lower() in (app.get_name() or "").lower():
            return app
    return None


def describe(o):
    try:
        role = o.get_role_name()
    except Exception as e:  # dead object
        return f"<dead {e}>"
    name = o.get_name() or ""
    desc = o.get_description() or ""
    ss = o.get_state_set()
    states = [s for s in INTERESTING if ss.contains(getattr(Atspi.StateType, s.upper()))]
    val = ""
    vi = o.get_value_iface() if hasattr(o, "get_value_iface") else None
    if vi is not None:
        try:
            val = f" value={vi.get_current_value():g} [{vi.get_minimum_value():g}..{vi.get_maximum_value():g}] step={vi.get_minimum_increment():g}"
            txt = vi.get_text() if hasattr(vi, "get_text") else None
            if txt:
                val += f" text={txt!r}"
        except Exception:
            pass
    acts = ""
    ai = o.get_action_iface() if hasattr(o, "get_action_iface") else None
    if ai is not None:
        try:
            acts = " actions=" + ",".join(ai.get_action_name(i) for i in range(ai.get_n_actions()))
        except Exception:
            pass
    d = f" desc={desc!r}" if desc else ""
    return f"{role} name={name!r}{d} states=[{','.join(states)}]{val}{acts}"


def walk(o, depth=0, out=None):
    out.append("  " * depth + describe(o))
    try:
        n = o.get_child_count()
    except Exception:
        return
    for i in range(n):
        c = o.get_child_at_index(i)
        if c is not None:
            walk(c, depth + 1, out)


def find_focused(o):
    try:
        if o.get_state_set().contains(Atspi.StateType.FOCUSED):
            return o
        for i in range(o.get_child_count()):
            c = o.get_child_at_index(i)
            if c is not None:
                r = find_focused(c)
                if r is not None:
                    return r
    except Exception:
        pass
    return None


if __name__ == "__main__":
    mode, sub = sys.argv[1], sys.argv[2]
    app = find_app(sub)
    if app is None:
        names = [Atspi.get_desktop(0).get_child_at_index(i).get_name() for i in range(Atspi.get_desktop(0).get_child_count())]
        print(f"APP NOT FOUND; registered apps: {names}")
        sys.exit(2)
    if mode == "tree":
        out = []
        walk(app, 0, out)
        print("\n".join(out))
    else:
        f = find_focused(app)
        print(describe(f) if f is not None else "NO FOCUSED OBJECT")
