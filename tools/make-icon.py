"""Draw collect_ui's icon: a blossom, as a union of five circles.

    nix-shell -p 'python3.withPackages (ps: [ ps.pillow ps.numpy ])' \
      --run 'python3 tools/make-icon.py \
                modules/collect_ui/src/icons/icon.png 55 65 70 20 0.333'

The source artwork was a 30x30 PNG and the platform wants 256x256, so it is
redrawn rather than upscaled — 8.5x of a bitmap is mush. The parameters were
measured off the original: outer radius 12 px, stroke 2 px (0.17 R), solid
centre 8 px across (0.33 R), five-fold with a petal pointing up.

A polar cosine curve (first attempt) pinches into cusps and reads as a star.
The real shape is five overlapping discs: convex petals meeting in shallow
concave fillets, which is what gives the soft flower outline. Working from a
distance field also gives exact antialiasing at 256 with no supersampling.
"""
import math, sys
import numpy as np
from PIL import Image

SIZE = 256
C = SIZE / 2.0


def union_sdf(x, y, petal_d, petal_r, core_r, phase=-math.pi / 2, petals=5):
    """Signed distance to the union of the petal discs and the centre disc."""
    sdf = np.hypot(x, y) - core_r
    for i in range(petals):
        a = phase + 2 * math.pi * i / petals
        sdf = np.minimum(sdf, np.hypot(x - petal_d * math.cos(a),
                                       y - petal_d * math.sin(a)) - petal_r)
    return sdf


def coverage(sdf):
    """Antialiased inside-ness: a one-pixel ramp across the boundary."""
    return np.clip(0.5 - sdf, 0.0, 1.0)


def render(out, petal_d, petal_r, core_r, stroke, seed_scale):
    ys, xs = np.mgrid[0:SIZE, 0:SIZE]
    x = xs + 0.5 - C
    y = ys + 0.5 - C

    outer = union_sdf(x, y, petal_d, petal_r, core_r)
    # The ring: inside the outline, outside the same shape eroded by `stroke`.
    # Eroding a distance field is just adding to it.
    ring = np.minimum(coverage(outer), coverage(-(outer + stroke)))

    # The solid centre: the same shape scaled down. It needs no clearance
    # term — the ring's inner edge is far outside it — and subtracting one
    # erased the centre entirely on the first try.
    s = seed_scale
    seed = union_sdf(x / s, y / s, petal_d, petal_r, core_r) * s
    alpha = np.maximum(ring, coverage(seed))

    rgba = np.zeros((SIZE, SIZE, 4), dtype=np.uint8)
    rgba[..., 3] = (np.clip(alpha, 0, 1) * 255).astype(np.uint8)
    Image.fromarray(rgba, "RGBA").save(out)
    print("wrote", out)


if __name__ == "__main__":
    out, *rest = sys.argv[1:]
    render(out, *[float(v) for v in rest])
