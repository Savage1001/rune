//! Retaining unchanged mounted [`memo`](crate::memo) subtrees across commits (`t726`).
//!
//! [`SubtreeRetention::FullRebuild`] runs every `memo` closure and flattens the full tree each
//! commit. [`SubtreeRetention::BuilderMemo`] reuses cached [`El`] subtrees in `memo`, then
//! expands and flattens them each commit. [`SubtreeRetention::RetainedMounted`] keeps the
//! flattened [`Node`] slices of unchanged mounted `memo` subtrees across commits, skipping
//! `El::expand_memo`, `flatten`, redundant `layout_entries` pruning, `path_ids` lookups,
//! unchanged text layout, and unchanged scene property updates.

use std::ops::Range;
use std::rc::Rc;

use super::*;

/// Sentinel role marking a [`Node`] slot in `Runtime::nodes` whose contents were moved into the
/// new commit's `nodes` vector during subtree splicing.
const STUB_ROLE: &str = "\0retained_stub";

/// Minimum table capacity retained before shrinking after a large view is replaced by a small one.
const FLOOR: usize = 64;

fn bound(live: usize) -> usize {
    (2 * live).max(FLOOR)
}

/// How [`Runtime`] retains unchanged mounted [`memo`](crate::memo) subtrees across commits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SubtreeRetention {
    /// Every commit runs every `memo` closure and rebuilds and flattens the full tree.
    FullRebuild,
    /// `memo` reuses cached `El` subtrees on version hits, then expands and flattens them each commit.
    BuilderMemo,
    /// `memo` reuses cached `El` subtrees and splices unchanged mounted `Node` subtrees across
    /// commits without expanding `El` children, re-flattening, re-laying-out unchanged text, or
    /// re-dirtying unchanged scene properties.
    #[default]
    RetainedMounted,
}

impl SubtreeRetention {
    pub(crate) fn caches_builder(self) -> bool {
        self != Self::FullRebuild
    }

    pub(crate) fn retains_mounted(self) -> bool {
        self == Self::RetainedMounted
    }
}

/// Counts from the most recent commit for [`Runtime::retained_subtree_stats`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RetainedSubtreeStats {
    /// Mounted `memo` subtrees currently retained for the next commit.
    pub retained_subtrees: usize,
    /// Mounted `memo` subtrees spliced without flattening in the last commit.
    pub spliced_subtrees: usize,
    /// Total [`Node`]s spliced without flattening in the last commit.
    pub spliced_nodes: usize,
    /// Allocated capacity (in entries) of the largest retention table.
    pub table_capacity: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FlattenContext {
    motion: Spring,
    in_row: RowLayout,
    parent_row: bool,
    parent_is_stack: bool,
    parent_instant_changes: bool,
    parent_trace_namespace: Option<(&'static str, &'static str)>,
    parent_inert: bool,
    parent_hover_within: Option<(Anim, Anim)>,
    scale_bits: u32,
}

impl FlattenContext {
    fn from_parent(parent: Option<usize>, motion: Spring, in_row: RowLayout, parent_row: bool, nodes: &[Node], scale: f32) -> Self {
        Self {
            motion,
            in_row,
            parent_row,
            parent_is_stack: parent.is_some_and(|p| nodes[p].growth.is_some()),
            parent_instant_changes: parent.is_some_and(|p| nodes[p].instant_changes),
            parent_trace_namespace: parent.and_then(|p| nodes[p].trace_namespace),
            parent_inert: parent.is_some_and(|p| nodes[p].inert),
            parent_hover_within: parent.and_then(|p| nodes[p].vis.hover_within),
            scale_bits: scale.to_bits(),
        }
    }
}

struct RetainedEntry {
    shared: Rc<crate::memo::Shared>,
    root: El,
    context: FlattenContext,
    node_range: Range<usize>,
    nested: Vec<usize>,
    taken: bool,
}

#[derive(Clone, Copy)]
struct SplicedRange {
    old_start: usize,
    new_start: usize,
    len: usize,
}

pub(super) struct RecordToken {
    ptr: usize,
    shared: Rc<crate::memo::Shared>,
    root: El,
    context: FlattenContext,
}

#[derive(Default)]
pub(crate) struct RetainedSubtrees {
    mode: SubtreeRetention,
    prev_entries: HashMap<usize, RetainedEntry>,
    next_entries: HashMap<usize, RetainedEntry>,
    spliced: Vec<SplicedRange>,
    nested_stack: Vec<Vec<usize>>,
    reused_entries: usize,
    pass_count: usize,
    last_spliced_subtrees: usize,
    last_spliced_nodes: usize,
}

pub(super) fn is_stub(n: &Node) -> bool {
    std::ptr::eq(n.role, STUB_ROLE)
}

fn stub_from(old: &Node) -> Node {
    Node {
        on_paste: None, on_window_focus: None, focus: crate::el::FocusSpec::default(), modal: None, looping: None,
        id: old.id, path: String::new(), key: None, trace_name: None, trace_namespace: None, role: STUB_ROLE,
        label: None, vis: Visual::default(), live: Vec::new(), text: None, text_layout: None, elided: None,
        run_layout: None, live_text_seen: None, on_press: None, on_double_press: None, on_secondary_press: None,
        on_hover: None, on_hover_change: None, hover_delay: [0.0; 2], on_pointer_move: None, gestures: None,
        above: None, on_drag: None, drag_out: None, drag_threshold: None, on_drag_start: None, on_drag_end: None,
        drop: None, motion: old.motion, opacity_motion: None, lead: None, width_leads: false,
        instant_changes: old.instant_changes, transition: None, matched: None, scroll: None,
        scroll_axes: crate::el::ScrollAxes::Y, on_at_bottom: None, on_far_from_bottom: None, on_visible_rows: None,
        on_layout: None, on_screen_layout: None, on_drawn_layout: None, on_scroll_offset: None, sticky: None,
        scroll_edges: crate::el::ScrollEdges::default(), growth: None, streaming: false, whole: false,
        input: None, cursor: None, window_drag: false, ime_cursor: false, selection_group: false,
        selection_copy: None, on_selection_change: None, a11y: crate::el::A11y::default(), inert: old.inert,
        press_scale: None, static_scale: old.static_scale, scale_track: old.scale_track, group_fade: old.group_fade,
        time_scale: old.time_scale, pixel_aspect: None, aspect_fit: None, material: None, in_row: old.in_row,
        virtual_mount: old.virtual_mount, parent: old.parent, taffy: old.taffy, target: old.target,
        local: old.local, inset: old.inset, help: None, help_cut: false,
        #[cfg(debug_assertions)]
        scroll_source: None,
    }
}

fn path_matches(full_path: &str, parent_path: &str, key: Option<&str>, index: usize) -> bool {
    match (parent_path.is_empty(), key) {
        (true, Some(k)) => full_path == k,
        (false, Some(k)) => {
            full_path.len() == parent_path.len() + 1 + k.len()
                && full_path.starts_with(parent_path)
                && full_path.as_bytes()[parent_path.len()] == b'/'
                && &full_path[parent_path.len() + 1..] == k
        }
        _ => full_path == node_path(parent_path, key, index),
    }
}

fn is_el_retainable(el: &El) -> bool {
    let auto_pixels = el.vis.pixels.is_some()
        && el.style.aspect_ratio.is_none()
        && (el.style.size.width.is_auto() != el.style.size.height.is_auto());
    el.key.as_deref().is_none_or(|k| !k.contains('/'))
        && el.virtual_estimate.is_none()
        && el.virtual_rows.is_none()
        && el.aspect_fit <= 0.0
        && el.above.is_none()
        && el.input.is_none()
        && el.matched.is_none()
        && !el.a11y.secure
        && !el.text.as_ref().is_some_and(|t| t.redacted)
        && !auto_pixels
        && !(el.scroll.is_some() && el.scroll_axes.x())
}

fn is_subtree_retainable(el: &El) -> bool {
    is_el_retainable(el) && el.all_children().all(is_subtree_retainable)
}

fn opt_rc_eq<T: ?Sized>(a: &Option<Rc<T>>, b: &Option<Rc<T>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

fn same_visual(a: &Visual, b: &Visual) -> bool {
    let same_surface = match (&a.surface, &b.surface) {
        (None, None) => true,
        (Some((sa, fa)), Some((sb, fb))) => sa.key() == sb.key() && fa == fb,
        _ => false,
    };
    let same_icon = match (&a.icon, &b.icon) {
        (None, None) => true,
        (Some((ia, ca)), Some((ib, cb))) => std::ptr::eq(*ia, *ib) && ca == cb,
        _ => false,
    };
    let same_pixels = match (&a.pixels, &b.pixels) {
        (None, None) => true,
        (Some(pa), Some(pb)) => Arc::ptr_eq(&pa.source, &pb.source),
        _ => false,
    };
    same_surface && same_icon && same_pixels && opt_rc_eq(&a.shape, &b.shape)
        && a.hit == b.hit && a.fill == b.fill && a.radius == b.radius
        && a.continuous_corners == b.continuous_corners && a.border_width == b.border_width
        && a.border == b.border && a.border_bottom == b.border_bottom
        && a.border_dash == b.border_dash && a.border_dash_phase == b.border_dash_phase
        && a.shadows == b.shadows && a.opacity == b.opacity && a.blur == b.blur
        && a.blur_motion == b.blur_motion && a.paint_alpha == b.paint_alpha && a.blend == b.blend
        && a.hover == b.hover && a.press == b.press && a.clip == b.clip
        && a.clip_radius == b.clip_radius && a.clip_corners == b.clip_corners
        && a.gradient == b.gradient && a.radial == b.radial && a.outline == b.outline
        && a.pixels_fit == b.pixels_fit && a.mark == b.mark && a.effect == b.effect
        && a.backdrop_blur == b.backdrop_blur && a.hover_group == b.hover_group
        && a.hover_motion == b.hover_motion && a.hover_within == b.hover_within
        && a.group_opacity == b.group_opacity && a.group_color == b.group_color
        && a.icon_turn == b.icon_turn && a.cache == b.cache && a.cache_key == b.cache_key
        && a.offset == b.offset && a.offset_motion == b.offset_motion && a.raise == b.raise
        && a.no_pixel_snap == b.no_pixel_snap && a.scale_motion == b.scale_motion
        && a.origin == b.origin && a.rotate == b.rotate && a.rotate_motion == b.rotate_motion
        && a.child_exit == b.child_exit && a.ride_trailing == b.ride_trailing
}

fn same_text(a: &Option<TextSpec>, b: &Option<TextSpec>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return a.is_none() && b.is_none();
    };
    let same_live = match (&a.live, &b.live) {
        (None, None) => true,
        (Some(la), Some(lb)) => Arc::ptr_eq(&la.source, &lb.source),
        _ => false,
    };
    let same_on_link = match (&a.on_link, &b.on_link) {
        (None, None) => true,
        (Some(la), Some(lb)) => Rc::ptr_eq(&la.0, &lb.0),
        _ => false,
    };
    let same_on_link_sec = match (&a.on_link_secondary, &b.on_link_secondary) {
        (None, None) => true,
        (Some(la), Some(lb)) => Rc::ptr_eq(&la.0, &lb.0),
        _ => false,
    };
    let same_adv = match (&a.advance_in, &b.advance_in) {
        (None, None) => true,
        (Some((ra, ia)), Some((rb, ib))) => ra.hash == rb.hash && ia == ib,
        _ => false,
    };
    let same_pills = a.pills.len() == b.pills.len()
        && a.pills.iter().zip(&b.pills).all(|(x, y)| {
            x.start == y.start && x.end == y.end && x.bg == y.bg && x.fg == y.fg
                && match (x.icon, y.icon) {
                    (None, None) => true,
                    (Some(ia), Some(ib)) => std::ptr::eq(ia, ib),
                    _ => false,
                }
        });
    same_live && same_on_link && same_on_link_sec && same_adv && same_pills
        && a.content == b.content && a.style == b.style && a.color == b.color
        && a.align == b.align && a.spans == b.spans && a.wrap == b.wrap
        && a.reveal == b.reveal && a.truncate == b.truncate && a.elide == b.elide
        && a.max_lines == b.max_lines && a.truncate_fade == b.truncate_fade
        && a.fade_em == b.fade_em && a.justify == b.justify && a.links == b.links
        && a.case == b.case && a.selectable == b.selectable && a.selection == b.selection
        && a.uncopied == b.uncopied && a.rings == b.rings && a.strikes == b.strikes
        && a.highlights == b.highlights && a.blocks == b.blocks && a.soft_breaks == b.soft_breaks
        && a.pill_colors == b.pill_colors && a.rich_style == b.rich_style
        && a.redacted == b.redacted && a.secure_revealed == b.secure_revealed
        && a.wrap_anywhere == b.wrap_anywhere
}

fn same_focus(a: &crate::el::FocusSpec, b: &crate::el::FocusSpec) -> bool {
    opt_rc_eq(&a.physical, &b.physical)
        && a.enabled == b.enabled && a.order == b.order && a.group == b.group
        && a.selected == b.selected && a.reveal_selected == b.reveal_selected
        && a.preserve_on_press.focus == b.preserve_on_press.focus
        && a.preserve_on_press.selection == b.preserve_on_press.selection
        && a.ring == b.ring && a.ring_off == b.ring_off && a.default == b.default
        && a.highlight == b.highlight && a.follows == b.follows && a.return_to == b.return_to
        && opt_rc_eq(&a.step, &b.step) && opt_rc_eq(&a.step_limit, &b.step_limit)
        && opt_rc_eq(&a.key_up, &b.key_up) && opt_rc_eq(&a.key, &b.key)
}

fn same_live_vec(a: &[crate::live::Bound], b: &[crate::live::Bound]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.property == y.property && x.spring == y.spring && Arc::ptr_eq(&x.source, &y.source)
        })
}

fn same_root_el(a: &El, b: &El) -> bool {
    let same_paste = match (&a.on_paste, &b.on_paste) {
        (None, None) => true,
        (Some(pa), Some(pb)) => pa.rich == pb.rich && Rc::ptr_eq(&pa.call, &pb.call),
        _ => false,
    };
    let same_modal = match (&a.modal, &b.modal) {
        (None, None) => true,
        (Some(ma), Some(mb)) => ma.outside == mb.outside && Rc::ptr_eq(&ma.dismiss, &mb.dismiss),
        _ => false,
    };
    let same_gestures = match (&a.gestures, &b.gestures) {
        (None, None) => true,
        (Some(ga), Some(gb)) => {
            ga.buttons == gb.buttons && opt_rc_eq(&ga.wheel, &gb.wheel)
                && opt_rc_eq(&ga.magnify, &gb.magnify) && opt_rc_eq(&ga.rotate, &gb.rotate)
                && opt_rc_eq(&ga.smart_magnify, &gb.smart_magnify)
        }
        _ => false,
    };
    let same_drop = match (&a.drop, &b.drop) {
        (None, None) => true,
        (Some(da), Some(db)) => {
            opt_rc_eq(&da.on_drop, &db.on_drop) && opt_rc_eq(&da.accepts, &db.accepts)
                && opt_rc_eq(&da.on_hover, &db.on_hover) && opt_rc_eq(&da.on_reject, &db.on_reject)
                && opt_rc_eq(&da.on_reject_hover, &db.on_reject_hover) && opt_rc_eq(&da.on_move, &db.on_move)
        }
        _ => false,
    };
    let same_far = match (&a.on_far_from_bottom, &b.on_far_from_bottom) {
        (None, None) => true,
        (Some((da, fa)), Some((db, fb))) => da == db && Rc::ptr_eq(fa, fb),
        _ => false,
    };
    a.children.is_empty() && b.children.is_empty() && a.role == b.role
        && a.label == b.label && a.trace_name == b.trace_name && a.trace_namespace == b.trace_namespace
        && a.style == b.style && same_visual(&a.vis, &b.vis) && same_live_vec(&a.live, &b.live)
        && same_text(&a.text, &b.text) && same_focus(&a.focus, &b.focus)
        && same_paste && same_modal && same_gestures && same_drop && same_far
        && opt_rc_eq(&a.on_window_focus, &b.on_window_focus) && opt_rc_eq(&a.on_press, &b.on_press)
        && opt_rc_eq(&a.on_double_press, &b.on_double_press)
        && opt_rc_eq(&a.on_secondary_press, &b.on_secondary_press)
        && opt_rc_eq(&a.on_hover, &b.on_hover) && opt_rc_eq(&a.on_hover_change, &b.on_hover_change)
        && opt_rc_eq(&a.on_pointer_move, &b.on_pointer_move) && opt_rc_eq(&a.on_drag, &b.on_drag)
        && opt_rc_eq(&a.drag_out, &b.drag_out) && opt_rc_eq(&a.on_drag_start, &b.on_drag_start)
        && opt_rc_eq(&a.on_drag_end, &b.on_drag_end) && opt_rc_eq(&a.on_at_bottom, &b.on_at_bottom)
        && opt_rc_eq(&a.on_visible_rows, &b.on_visible_rows) && opt_rc_eq(&a.on_layout, &b.on_layout)
        && opt_rc_eq(&a.on_screen_layout, &b.on_screen_layout)
        && opt_rc_eq(&a.on_drawn_layout, &b.on_drawn_layout)
        && opt_rc_eq(&a.on_scroll_offset, &b.on_scroll_offset)
        && opt_rc_eq(&a.selection_copy, &b.selection_copy)
        && opt_rc_eq(&a.on_selection_change, &b.on_selection_change)
        && a.looping == b.looping && a.hover_delay == b.hover_delay && a.above == b.above
        && a.drag_threshold == b.drag_threshold && a.motion == b.motion
        && a.opacity_motion == b.opacity_motion && a.lead == b.lead && a.width_leads == b.width_leads
        && a.animate_layout == b.animate_layout && a.instant_changes == b.instant_changes
        && a.transition == b.transition && a.matched == b.matched && a.scroll == b.scroll
        && a.scroll_axes == b.scroll_axes && a.sticky == b.sticky && a.scroll_edges == b.scroll_edges
        && a.growth == b.growth && a.virtual_estimate == b.virtual_estimate
        && a.streaming == b.streaming && a.whole == b.whole && a.cursor == b.cursor
        && a.window_drag == b.window_drag && a.ime_cursor == b.ime_cursor
        && a.press_scale == b.press_scale && a.scale == b.scale && a.time_scale == b.time_scale
        && a.help == b.help && a.help_cut == b.help_cut && a.material == b.material
        && a.aspect_fit == b.aspect_fit && a.selection_group == b.selection_group
        && a.a11y == b.a11y && a.inert == b.inert
}

impl RetainedSubtrees {
    pub(crate) fn mode(&self) -> SubtreeRetention {
        self.mode
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.mode.retains_mounted()
    }

    pub(crate) fn set_mode(&mut self, mode: SubtreeRetention) {
        self.mode = mode;
        if !mode.retains_mounted() {
            self.clear();
        }
    }

    pub(crate) fn clear(&mut self) {
        self.prev_entries = HashMap::new();
        self.next_entries = HashMap::new();
        self.spliced = Vec::new();
        self.nested_stack = Vec::new();
        self.reused_entries = 0;
        self.pass_count = 0;
        self.last_spliced_subtrees = 0;
        self.last_spliced_nodes = 0;
    }

    pub(crate) fn stats(&self) -> RetainedSubtreeStats {
        RetainedSubtreeStats {
            retained_subtrees: self.prev_entries.len(),
            spliced_subtrees: self.last_spliced_subtrees,
            spliced_nodes: self.last_spliced_nodes,
            table_capacity: self.prev_entries.capacity().max(self.next_entries.capacity()),
        }
    }

    pub(super) fn begin_commit(&mut self, old_nodes: &[Node], scene: &Scene) {
        self.pass_count = 0;
        self.reused_entries = 0;
        self.spliced.clear();
        self.nested_stack.clear();
        self.next_entries.clear();
        if !self.is_enabled() {
            if !self.prev_entries.is_empty() {
                self.prev_entries.clear();
            }
            return;
        }
        self.prev_entries.retain(|_, entry| {
            entry.taken = false;
            entry.node_range.start < entry.node_range.end
                && entry.node_range.end <= old_nodes.len()
                && old_nodes[entry.node_range.clone()].iter().all(|n| {
                    !is_stub(n)
                        && scene.contains(n.id)
                        && !matches!(scene.phase(n.id), Some(PresencePhase::Exiting { .. }))
                })
        });
    }

    pub(super) fn begin_measure_pass(&mut self) {
        self.pass_count += 1;
        self.reused_entries = 0;
        self.spliced.clear();
        self.nested_stack.clear();
        self.next_entries.clear();
        for entry in self.prev_entries.values_mut() {
            entry.taken = false;
        }
    }

    fn is_retainable(el: &El, shared: &crate::memo::Shared) -> bool {
        if !is_el_retainable(el) {
            return false;
        }
        match shared.retainable.get() {
            0 => {
                let ok = shared.children.iter().all(is_subtree_retainable);
                shared.retainable.set(if ok { 2 } else { 1 });
                ok
            }
            v => v == 2,
        }
    }

    pub(super) fn skip_window_tree(&self, el: &El) -> bool {
        if !self.is_enabled() || !el.children.is_empty() || el.virtual_estimate.is_some() || el.virtual_rows.is_some() {
            return false;
        }
        let Some(shared) = &el.memo_children else {
            return false;
        };
        Self::is_retainable(el, shared)
    }

    pub(super) fn note_entry_reused(&mut self) {
        self.reused_entries += 1;
    }

    pub(super) fn prepare_flatten(&mut self, entries: &mut HashMap<String, LayoutEntry>) -> usize {
        if !self.is_enabled() {
            for entry in entries.values_mut() {
                entry.used = false;
            }
        }
        entries.len()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn try_splice(
        &mut self,
        el: &El,
        parent: Option<usize>,
        parent_path: &str,
        index: usize,
        motion: Spring,
        in_row: RowLayout,
        parent_row: bool,
        scale: f32,
        taffy: &mut TaffyTree<usize>,
        nodes: &mut Vec<Node>,
        old_nodes: &mut [Node],
        entries: &HashMap<String, LayoutEntry>,
    ) -> Option<taffy::NodeId> {
        if !self.is_enabled() || !el.children.is_empty() {
            return None;
        }
        let shared = el.memo_children.as_ref()?;
        let ptr = Rc::as_ptr(shared) as usize;
        let ctx = FlattenContext::from_parent(parent, motion, in_row, parent_row, nodes, scale);
        let (old_range, nested) = {
            let entry = self.prev_entries.get_mut(&ptr)?;
            if entry.taken || !Rc::ptr_eq(&entry.shared, shared) || entry.context != ctx {
                return None;
            }
            let r = entry.node_range.clone();
            if r.start >= r.end || r.end > old_nodes.len() || is_stub(&old_nodes[r.start]) {
                return None;
            }
            if el.key != old_nodes[r.start].key || !path_matches(&old_nodes[r.start].path, parent_path, el.key.as_deref(), index) {
                return None;
            }
            let entries_ok = if self.pass_count <= 1 {
                entries.get(&old_nodes[r.start].path).is_some_and(|e| e.id == old_nodes[r.start].taffy)
            } else {
                old_nodes[r.clone()].iter().all(|n| entries.get(&n.path).is_some_and(|e| e.id == n.taffy))
            };
            if !entries_ok || !same_root_el(el, &entry.root) {
                return None;
            }
            entry.taken = true;
            (r, entry.nested.clone())
        };

        let old_start = old_range.start;
        let len = old_range.end - old_start;
        let new_start = nodes.len();
        let new_end = new_start + len;
        let delta = new_start as isize - old_start as isize;
        let need_ctx_update = delta != 0 || self.pass_count > 1;

        for k in 0..len {
            let oi = old_start + k;
            let ni = new_start + k;
            let stub = stub_from(&old_nodes[oi]);
            let mut node = std::mem::replace(&mut old_nodes[oi], stub);
            node.parent = if k == 0 { parent } else { node.parent.map(|p| (p as isize + delta) as usize) };
            node.virtual_mount = k == 0 && el.virtual_mount;
            if need_ctx_update && node.text.is_some()
                && let Some(c) = taffy.get_node_context_mut(node.taffy)
                && *c != ni
            {
                *c = ni;
            }
            nodes.push(node);
        }

        let root_taffy = nodes[new_start].taffy;
        self.reused_entries += len;
        self.spliced.push(SplicedRange { old_start, new_start, len });

        if let Some(parent_nested) = self.nested_stack.last_mut() {
            parent_nested.push(ptr);
            parent_nested.extend_from_slice(&nested);
        }
        for &nptr in &nested {
            if let Some(ne) = self.prev_entries.get_mut(&nptr) {
                ne.taken = true;
                let ns = (ne.node_range.start as isize + delta) as usize;
                let ne_end = (ne.node_range.end as isize + delta) as usize;
                self.next_entries.insert(
                    nptr,
                    RetainedEntry {
                        shared: ne.shared.clone(),
                        root: ne.root.clone(),
                        context: ne.context,
                        node_range: ns..ne_end,
                        nested: ne.nested.clone(),
                        taken: false,
                    },
                );
            }
        }
        if let Some(entry) = self.prev_entries.get(&ptr) {
            self.next_entries.insert(
                ptr,
                RetainedEntry {
                    shared: entry.shared.clone(),
                    root: entry.root.clone(),
                    context: ctx,
                    node_range: new_start..new_end,
                    nested,
                    taken: false,
                },
            );
        }
        Some(root_taffy)
    }

    pub(super) fn prepare_record(
        &mut self,
        el: &mut El,
        parent: Option<usize>,
        motion: Spring,
        in_row: RowLayout,
        parent_row: bool,
        nodes: &[Node],
        scale: f32,
    ) -> Option<RecordToken> {
        if !self.is_enabled() || !el.children.is_empty() {
            return None;
        }
        let shared = el.memo_children.as_ref()?.clone();
        if !Self::is_retainable(el, &shared) {
            return None;
        }
        let ptr = Rc::as_ptr(&shared) as usize;
        let context = FlattenContext::from_parent(parent, motion, in_row, parent_row, nodes, scale);
        let key = el.key.take();
        let mut root = el.clone();
        el.key = key;
        root.memo_children = None;
        self.nested_stack.push(Vec::new());
        Some(RecordToken { ptr, shared, root, context })
    }

    pub(super) fn finish_record(&mut self, token: RecordToken, node_range: Range<usize>) {
        let nested = self.nested_stack.pop().unwrap_or_default();
        if let Some(parent_nested) = self.nested_stack.last_mut() {
            parent_nested.push(token.ptr);
            parent_nested.extend_from_slice(&nested);
        }
        self.next_entries.insert(
            token.ptr,
            RetainedEntry {
                shared: token.shared,
                root: token.root,
                context: token.context,
                node_range,
                nested,
                taken: false,
            },
        );
    }

    pub(super) fn finish_flatten(
        &mut self,
        prev_entries: usize,
        entries: &mut HashMap<String, LayoutEntry>,
        taffy: &mut TaffyTree<usize>,
        nodes: &[Node],
    ) {
        if !self.is_enabled() {
            entries.retain(|_, entry| {
                if !entry.used {
                    taffy.remove(entry.id).unwrap();
                }
                entry.used
            });
            return;
        }
        if self.reused_entries < prev_entries {
            for entry in entries.values_mut() {
                entry.used = false;
            }
            for n in nodes {
                if let Some(entry) = entries.get_mut(&n.path) {
                    entry.used = true;
                }
            }
            entries.retain(|_, entry| {
                if !entry.used {
                    taffy.remove(entry.id).unwrap();
                }
                entry.used
            });
        }
    }

    pub(super) fn restore_spliced(&mut self, old_nodes: &mut [Node], nodes: &mut [Node]) {
        self.prev_entries.retain(|_, entry| entry.taken);
        for range in self.spliced.drain(..).rev() {
            for k in 0..range.len {
                let oi = range.old_start + k;
                let ni = range.new_start + k;
                let stub = &old_nodes[oi];
                let (parent, target, local, inset, virtual_mount) =
                    (stub.parent, stub.target, stub.local, stub.inset, stub.virtual_mount);
                let old_w = (local.w - inset[0] - inset[2]).max(0.0);
                let new_w = (nodes[ni].local.w - nodes[ni].inset[0] - nodes[ni].inset[2]).max(0.0);
                let mut node = std::mem::replace(&mut nodes[ni], stub_from(stub));
                if (old_w - new_w).abs() > 1e-4 {
                    node.text_layout = None;
                    node.elided = None;
                }
                node.parent = parent;
                node.target = target;
                node.local = local;
                node.inset = inset;
                node.virtual_mount = virtual_mount;
                old_nodes[oi] = node;
            }
        }
    }

    /// If node `i` in `nodes` was spliced in this commit, returns `(old_index_in_self_nodes, subtree_root_in_nodes)`.
    pub(super) fn spliced_at(&self, i: usize, cursor: &mut usize) -> Option<(usize, usize)> {
        while *cursor < self.spliced.len() && i >= self.spliced[*cursor].new_start + self.spliced[*cursor].len {
            *cursor += 1;
        }
        let s = self.spliced.get(*cursor)?;
        (i >= s.new_start).then(|| (s.old_start + (i - s.new_start), s.new_start))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn skip_visual_apply(
        &self,
        is_spliced: bool,
        instant: bool,
        lands: bool,
        out_of_sight: bool,
        reduced_motion_changed: bool,
        tx: &Tx,
        scene: &Scene,
    ) -> bool {
        is_spliced && !instant && !lands && !out_of_sight && !reduced_motion_changed && !tx.settle && !scene.tracing()
    }

    pub(super) fn finish_commit(&mut self, live_nodes: usize) {
        if !self.is_enabled() {
            self.last_spliced_subtrees = 0;
            self.last_spliced_nodes = 0;
            return;
        }
        self.last_spliced_subtrees = self.spliced.len();
        self.last_spliced_nodes = self.spliced.iter().map(|s| s.len).sum();
        self.spliced.clear();
        self.nested_stack.clear();
        std::mem::swap(&mut self.prev_entries, &mut self.next_entries);
        self.next_entries.clear();

        let cap = bound(self.prev_entries.len().min(live_nodes));
        if self.prev_entries.capacity() > cap {
            self.prev_entries.shrink_to(cap);
        }
        if self.next_entries.capacity() > cap {
            self.next_entries.shrink_to(cap);
        }
        if self.spliced.capacity() > cap {
            self.spliced.shrink_to(cap);
        }
    }

    pub(super) fn remap_after_prune(&mut self, remap: &[Option<usize>]) {
        if !self.is_enabled() || self.prev_entries.is_empty() {
            return;
        }
        self.prev_entries.retain(|_, entry| {
            let r = &entry.node_range;
            if r.start >= r.end || r.end > remap.len() {
                return false;
            }
            let (Some(ns), Some(ne_last)) = (remap[r.start], remap[r.end - 1]) else {
                return false;
            };
            let len = r.len();
            if ne_last + 1 != ns + len {
                return false;
            }
            entry.node_range = ns..(ns + len);
            true
        });
    }
}

impl<S: 'static> Runtime<S> {
    /// Chooses how [`memo`](crate::memo) subtrees are retained across commits.
    pub fn set_subtree_retention(&mut self, mode: SubtreeRetention) {
        self.retained.set_mode(mode);
    }

    /// Current [`SubtreeRetention`] mode.
    pub fn subtree_retention(&self) -> SubtreeRetention {
        self.retained.mode()
    }

    /// Subtree retention counts from the most recent commit.
    pub fn retained_subtree_stats(&self) -> RetainedSubtreeStats {
        self.retained.stats()
    }
}

#[cfg(test)]
#[path = "retained_subtrees_tests.rs"]
mod tests;
