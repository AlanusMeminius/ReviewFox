"""Even out the optical size of Lucide icons that sit side by side in a toolbar.

Lucide glyphs share a 24 grid but not an ink size: an arrow covers 14x14, a
fold icon 20x20, so in one row some look small and some crowd their box. This
fits each file's viewBox so the geometry's bounding box averages TARGET of the
grid (capped at MAX_SIDE on the longer side) and is centred, then scales
stroke-width with the viewBox so every icon still renders at the same line
weight (2/24 of the icon size).

It reads the path coordinates, which it never rewrites, so running it again on
an already-fitted file gives the same result.

Usage, from the repo root, on the icons that share a row:

    python tools/optical.py assets/search.svg assets/arrow_up.svg ...

Currently fitted: the Diff toolbar (sidebar_title, arrow_up, arrow_down,
unfold_vertical, fold_vertical, pilcrow, search, export,
square_minus, square_plus, diff) and the main toolbar (diff_title, gear,
folder). Do not run it on minus/plus: a flat line has no height to average,
so they stay at Lucide's own size.

Handles <path>, <rect> and <circle>. Bezier extents use their control points,
which can only overestimate the box slightly.
"""

import math
import re
import sys

TARGET = 18.0  # mean of ink width and height, in 24-grid units
MAX_SIDE = 21.0  # the longer side never crowds the box edge

ARG_COUNTS = {"M": 2, "L": 2, "H": 1, "V": 1, "C": 6, "S": 4, "Q": 4, "T": 2, "A": 7, "Z": 0}


def path_points(d):
    tokens = re.findall(r"[A-Za-z]|-?(?:\d+\.?\d*|\.\d+)(?:e-?\d+)?", d)
    points = []
    i = 0
    cmd = None
    x = y = start_x = start_y = 0.0

    while i < len(tokens):
        if tokens[i].isalpha():
            cmd = tokens[i]
            i += 1
        elif cmd is None:
            raise ValueError(f"path data starts without a command: {d}")
        upper = cmd.upper()
        rel = cmd.islower()
        if upper == "Z":
            x, y = start_x, start_y
            points.append((x, y))
            continue
        args = [float(t) for t in tokens[i : i + ARG_COUNTS[upper]]]
        i += ARG_COUNTS[upper]
        if upper == "H":
            x = x + args[0] if rel else args[0]
        elif upper == "V":
            y = y + args[0] if rel else args[0]
        elif upper == "A":
            end_x, end_y = (x + args[5], y + args[6]) if rel else (args[5], args[6])
            points += arc_points(x, y, *args[:5], end_x, end_y)
            x, y = end_x, end_y
        else:
            for k in range(0, len(args) - 2, 2):
                points.append((x + args[k], y + args[k + 1]) if rel else (args[k], args[k + 1]))
            x, y = (x + args[-2], y + args[-1]) if rel else (args[-2], args[-1])
        points.append((x, y))
        if upper == "M":
            start_x, start_y = x, y
            # Further coordinate pairs after a moveto are implicit linetos.
            cmd = "l" if rel else "L"
    return points


def arc_points(x1, y1, rx, ry, phi, large_arc, sweep, x2, y2, samples=32):
    """Sample an SVG elliptical arc (endpoint parameterization, SVG spec F.6.5)."""
    if rx == 0 or ry == 0:
        return []
    rx, ry = abs(rx), abs(ry)
    cos_phi, sin_phi = math.cos(math.radians(phi)), math.sin(math.radians(phi))
    dx, dy = (x1 - x2) / 2, (y1 - y2) / 2
    xp, yp = cos_phi * dx + sin_phi * dy, -sin_phi * dx + cos_phi * dy
    radii_scale = xp * xp / (rx * rx) + yp * yp / (ry * ry)
    if radii_scale > 1:
        rx *= math.sqrt(radii_scale)
        ry *= math.sqrt(radii_scale)
    num = rx * rx * ry * ry - rx * rx * yp * yp - ry * ry * xp * xp
    den = rx * rx * yp * yp + ry * ry * xp * xp
    k = math.sqrt(max(0.0, num / den)) * (-1 if large_arc == sweep else 1)
    cxp, cyp = k * rx * yp / ry, -k * ry * xp / rx
    cx = cos_phi * cxp - sin_phi * cyp + (x1 + x2) / 2
    cy = sin_phi * cxp + cos_phi * cyp + (y1 + y2) / 2

    def angle(ux, uy, vx, vy):
        return math.atan2(ux * vy - uy * vx, ux * vx + uy * vy)

    ux, uy = (xp - cxp) / rx, (yp - cyp) / ry
    start = angle(1, 0, ux, uy)
    delta = angle(ux, uy, (-xp - cxp) / rx, (-yp - cyp) / ry)
    if not sweep and delta > 0:
        delta -= 2 * math.pi
    if sweep and delta < 0:
        delta += 2 * math.pi

    points = []
    for n in range(samples + 1):
        t = start + delta * n / samples
        ex, ey = rx * math.cos(t), ry * math.sin(t)
        points.append((cx + ex * cos_phi - ey * sin_phi, cy + ex * sin_phi + ey * cos_phi))
    return points


def attrs(tag):
    return dict(re.findall(r'([\w-]+)="([^"]*)"', tag))


def ink_box(svg):
    points = []
    for name, rest in re.findall(r"<(path|rect|circle)\b([^>]*)>", svg):
        a = attrs(rest)
        if name == "path":
            points += path_points(a["d"])
        elif name == "rect":
            x, y, w, h = (float(a.get(k, 0)) for k in ("x", "y", "width", "height"))
            points += [(x, y), (x + w, y + h)]
        else:
            cx, cy, r = (float(a[k]) for k in ("cx", "cy", "r"))
            points += [(cx - r, cy - r), (cx + r, cy + r)]
    if not points:
        raise ValueError("no <path>, <rect> or <circle> to measure")
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    return min(xs), min(ys), max(xs), max(ys)


def fit(path):
    svg = open(path, encoding="utf-8").read()
    x0, y0, x1, y1 = ink_box(svg)
    w, h = x1 - x0, y1 - y0
    scale = min(TARGET / ((w + h) / 2), MAX_SIDE / max(w, h))
    side = 24 / scale
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    view_box = f"{cx - side / 2:.3g} {cy - side / 2:.3g} {side:.4g} {side:.4g}"
    stroke = f"{2 * side / 24:.3g}"
    svg = re.sub(r' viewBox="[^"]*"', f' viewBox="{view_box}"', svg, count=1)
    svg = re.sub(r'stroke-width="[^"]*"', f'stroke-width="{stroke}"', svg)
    open(path, "w", encoding="utf-8", newline="\n").write(svg)
    print(f"{path}: ink {w:.1f}x{h:.1f} -> {w * scale:.1f}x{h * scale:.1f}, viewBox {view_box}, stroke {stroke}")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    for arg in sys.argv[1:]:
        fit(arg)
