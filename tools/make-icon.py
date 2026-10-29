"""Draw collect_ui's icon: a blossom outline with a five-pointed centre.

    nix-shell -p 'python3.withPackages (ps: [ ps.pillow ps.numpy ])' \
      --run 'python3 tools/make-icon.py modules/collect_ui/src/icons/icon.png'

The source artwork is 30x30 and the platform wants 256x256, so it is redrawn
rather than upscaled — 8.5x of a bitmap is mush.

Everything below is measured off the original by probing its alpha along rays
from the centre, not guessed. Per ray the probe finds four edges: the centre
mark, then the inner and outer edges of the ring. In source pixels, with the
outer radius peaking at 13.4:

    outer tip      13.4        valley   9.7     -> lobes are 28% deep
    stroke          2.2                         -> 0.16 of the outer radius
    centre tip      4.8        valley   3.1     -> 0.36 and 0.23 of it

Two different constructions, because the two parts are different shapes:

* **The ring is a union of five discs.** Convex petals meeting in shallow
  concave fillets is what gives the soft outline. A polar cosine curve was the
  first attempt and pinches into cusps that read as a star.
* **The centre is a polar cosine curve**, because there it *should* read as a
  star — which is exactly what the original's centre mark is.
"""

import math
import sys

import numpy as np
from PIL import Image

SIZE = 256
C = SIZE / 2.0
PHASE = -math.pi / 2  # a petal points up
PETALS = 5

# Geometry in canvas pixels, from the measurements above scaled by 256/30.
OUTER = 120.0          # radius at a petal tip; leaves a small margin
PETAL_D = 69.0         # distance from centre to each disc's centre
PETAL_R = OUTER - PETAL_D
CORE_R = 78.0          # fills the middle so the petals form one shape
# 78 and not the ~58 that merely connects them: the ring is drawn by eroding
# this union by STROKE, and an eroded union of barely-touching discs breaks
# into five separate rings. A core this size keeps the inner opening one
# smooth shape. It cannot distort the outline — the outer valley sits at 87,
# well beyond it.
STROKE = 20.0
SEED_TIP = 43.0
SEED_DEPTH = 0.18      # (tip - valley) / tip, from 4.8 and 3.1


def ring_sdf(x, y):
    """Signed distance to the union of the petal discs and the centre disc."""
    sdf = np.hypot(x, y) - CORE_R
    for i in range(PETALS):
        a = PHASE + 2 * math.pi * i / PETALS
        sdf = np.minimum(
            sdf,
            np.hypot(x - PETAL_D * math.cos(a), y - PETAL_D * math.sin(a)) - PETAL_R,
        )
    return sdf


def star_sdf(x, y):
    """Approximate signed distance to the polar curve r = r0 + r1*cos(5t).

    Radial difference rather than true distance: the curve is smooth and
    shallow, so over the one pixel that antialiasing cares about the two agree
    closely enough.
    """
    r1 = SEED_TIP * SEED_DEPTH
    r0 = SEED_TIP - r1
    t = np.arctan2(y, x)
    return np.hypot(x, y) - (r0 + r1 * np.cos(PETALS * (t - PHASE)))


def coverage(sdf):
    """Antialiased inside-ness: a one-pixel ramp across the boundary."""
    return np.clip(0.5 - sdf, 0.0, 1.0)


def render(out):
    ys, xs = np.mgrid[0:SIZE, 0:SIZE]
    x = xs + 0.5 - C
    y = ys + 0.5 - C

    outline = ring_sdf(x, y)
    # The ring: inside the outline, outside the same shape eroded by the
    # stroke. Eroding a distance field is just adding to it.
    ring = np.minimum(coverage(outline), coverage(-(outline + STROKE)))
    alpha = np.maximum(ring, coverage(star_sdf(x, y)))

    rgba = np.zeros((SIZE, SIZE, 4), dtype=np.uint8)
    rgba[..., 3] = (np.clip(alpha, 0.0, 1.0) * 255).astype(np.uint8)
    Image.fromarray(rgba, "RGBA").save(out)
    print("wrote", out)


if __name__ == "__main__":
    render(sys.argv[1] if len(sys.argv) > 1 else "icon.png")
