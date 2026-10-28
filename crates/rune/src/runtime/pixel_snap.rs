//! Paint-time pixel snapping (`El::pixel_snap`). Layout stays fractional; only where a box's
//! fill, border and clips land on the device grid changes, so a 1.1 pt gap at 2× still draws
//! hard edges.
//!
//! Edges snap, not origin plus size: each of a box's four edges rounds to the nearest device
//! pixel, so two boxes that share an edge round it the same way and meet with no seam or
//! overlap. A box at least half a pixel across keeps at least one pixel; a thinner one keeps
//! its exact edges on that axis rather than vanish.
//!
//! Motion: a moving box's edges are corrected toward the rounding of where it comes to rest
//! (every track at its target), not of where it is, so it glides at full sub-pixel precision
//! with no stepping. When the rest changes, each edge eases from the correction it drew with
//! at that moment to the new rest's, in proportion to how far it has left to travel, so a
//! motion starts where the box was and its last frame is the snapped resting rect: no jump at
//! either end. Boxes that move together share their progress, so a child keeps its place
//! against its parent's edges as both glide. Translations that are not animation tracks
//! (scroll offsets, sticky shifts, rows growing above) move a whole subtree together: they snap
//! as one rounded translation, so the rows of a scrolling list stay crisp and keep their
//! spacing every frame. A scaled subtree doesn't snap while the scale moves its edges by a
//! pixel or more; over the last pixel of a scale settling at 1 the snap fades in, so it lands
//! without a jump either. Turned subtrees don't snap.

use std::collections::HashMap;

use rune_motion::{NodeId, Presented, Rect};

/// How far each edge of a moving node is from where it comes to rest (device px), and that
/// resting rect (points), which keys its glide.
#[derive(Clone, Copy)]
pub(super) struct Motion {
    by: [f32; 4],
    rest: [f32; 4],
}

/// `p`'s motion toward `rest`, the rect the scene gives while the node is moving. An anchored
/// element is placed from its anchor, not its frame, so its frame's motion doesn't move it; a
/// growth row draws its bottom edge at its top plus its current height.
pub(super) fn motion(p: &Presented, rest: Option<Rect>, s: f32, anchored: bool, growth_row: bool) -> Option<Motion> {
    let rest = rest.filter(|_| !anchored)?;
    let (a, b) = (p.rect.edges(), rest.edges());
    let mut by: [f32; 4] = core::array::from_fn(|k| (a[k] - b[k]) * s);
    if growth_row {
        by[3] = by[1];
    }
    Some(Motion { by, rest: b })
}

/// One moving node's glide: its edges' corrections go from `from` to `to` (the resting rect's
/// rounding) as their motion goes from `span` to 0.
#[derive(Clone, Copy)]
struct Glide {
    rest: [f32; 4],
    /// Resting edges in device px, less the rigid translation.
    rest_px: [f32; 4],
    from: [f32; 4],
    to: [f32; 4],
    span: [f32; 4],
    seen: u32,
}

impl Glide {
    /// Edge `k`'s correction with `m` px left to travel. Past the start (a retarget that carries
    /// it further away) it holds the starting correction.
    fn at(&self, k: usize, to: f32, m: f32) -> f32 {
        let t = if self.span[k].abs() > 1.0 / 64.0 { (m / self.span[k]).clamp(-1.0, 1.0) } else { 0.0 };
        to + (self.from[k] - to) * t
    }
}

/// The glides of the nodes moving now, owned by the runtime.
#[derive(Default)]
pub(super) struct Glides {
    glides: HashMap<NodeId, Glide>,
    frame: u32,
}

/// Frames a glide outlives its node's last paint (a cached subtree, a removed node).
const KEEP: u32 = 120;

impl Glides {
    /// Called once before each paint pass.
    pub(super) fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        if !self.glides.is_empty() {
            let frame = self.frame;
            self.glides.retain(|_, g| frame.wrapping_sub(g.seen) < KEEP);
        }
    }

    /// Node `id` is at rest and unscaled: its paint rect `r` (device px edges) snapped, with
    /// `rigid` from `Inh::snap_rigid` in device px.
    #[inline]
    pub(super) fn at_rest(&mut self, id: NodeId, r: [f32; 4], rigid: [f32; 2]) -> [f32; 4] {
        if !self.glides.is_empty() {
            self.glides.remove(&id);
        }
        snap(r, [0.0; 4], rigid, |_, c| c)
    }

    /// Node `id`'s paint rect `r` (device px edges) snapped. `unscaled` is the same rect before
    /// the scales on it and above it, `motion` from [`motion`], and `rigid` as in
    /// [`Self::at_rest`].
    pub(super) fn snapped(&mut self, id: NodeId, motion: Option<Motion>, (r, unscaled): ([f32; 4], [f32; 4]), rigid: [f32; 2]) -> [f32; 4] {
        // How far scaling moves the edges. Snapping fades in over its last pixel, so a scale
        // settling at 1 lands on the grid without a jump; anything scaled further stays exact.
        let moved = (0..4).map(|k| (r[k] - unscaled[k]).abs()).fold(0.0, f32::max);
        let weight = ((1.0 - moved) / (1.0 - 1.0 / 16.0)).clamp(0.0, 1.0);
        if weight == 0.0 {
            return r;
        }
        let snapped = match motion {
            None => self.at_rest(id, unscaled, rigid),
            Some(m) => self.glide(id, m, unscaled, rigid),
        };
        core::array::from_fn(|k| r[k] + (snapped[k] - r[k]) * weight)
    }

    fn glide(&mut self, id: NodeId, m: Motion, r: [f32; 4], rigid: [f32; 2]) -> [f32; 4] {
        let free: [f32; 4] = core::array::from_fn(|k| r[k] - rigid[k % 2]);
        let mut g = match self.glides.get(&id) {
            Some(g) if g.rest == m.rest => *g,
            prev => {
                // A new rest: start from the correction drawn with now, so nothing jumps.
                let from = match prev {
                    Some(g) => core::array::from_fn(|k| g.at(k, g.to[k], free[k] - g.rest_px[k])),
                    None => {
                        let mut now = [0.0; 4];
                        snap(r, [0.0; 4], rigid, |k, c| {
                            now[k] = c;
                            c
                        });
                        now
                    }
                };
                Glide { rest: m.rest, rest_px: core::array::from_fn(|k| free[k] - m.by[k]), from, to: from, span: m.by, seen: 0 }
            }
        };
        let out = snap(r, m.by, rigid, |k, c| {
            g.to[k] = c;
            g.at(k, c, m.by[k])
        });
        g.seen = self.frame;
        self.glides.insert(id, g);
        out
    }
}

/// Rounds half up on the device grid, the same way at every position.
fn round(v: f32) -> f32 {
    (v + 0.5).floor()
}

/// `r` (device px edges, `x0, y0, x1, y1`) snapped. `motion` is how far each edge is from where
/// it comes to rest and `rigid` the translation that snaps as a whole (both device px). Each
/// edge's correction toward its resting rounding passes through `ease(edge, correction)`.
#[inline]
pub(super) fn snap(r: [f32; 4], motion: [f32; 4], rigid: [f32; 2], mut ease: impl FnMut(usize, f32) -> f32) -> [f32; 4] {
    let mut out = r;
    for axis in 0..2 {
        let (a, b) = (axis, axis + 2);
        let (rest_a, rest_b) = (r[a] - motion[a] - rigid[axis], r[b] - motion[b] - rigid[axis]);
        if rest_b - rest_a < 0.5 {
            continue;
        }
        let shift = round(rigid[axis]);
        let lo = round(rest_a);
        let hi = round(rest_b).max(lo + 1.0);
        for (k, rest, at) in [(a, rest_a, lo), (b, rest_b, hi)] {
            // Built from the rounded edge, so a box at rest lands exactly on whole pixels.
            let (c, eased) = (at - rest, ease(k, at - rest));
            out[k] = if eased == c { at } else { rest + eased } + shift + motion[k];
        }
    }
    out
}
