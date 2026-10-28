//! The entry points that lay out rich text: [`TextSystem::layout_rich`] and its aligned,
//! block-indented and per-spec variants, all over `layout_blocks_inner`.
use super::*;

impl TextSystem {
    /// A layout in device pixels, wrapped at `max_width` device px when given.
    pub fn layout_rich(
        &mut self,
        text: &str,
        spans: &[Span],
        style: TextStyle,
        scale: f32,
        max_width: Option<f32>,
    ) -> Rc<Layout<Ink>> {
        self.layout_aligned(text, spans, style, scale, max_width, Alignment::Start)
    }

    /// [`layout_rich`](Self::layout_rich) with each line aligned within `max_width`, so wrapped
    /// centred or end-aligned text lines up per line rather than as one left-aligned block.
    pub fn layout_aligned(
        &mut self,
        text: &str,
        spans: &[Span],
        style: TextStyle,
        scale: f32,
        max_width: Option<f32>,
        align: Alignment,
    ) -> Rc<Layout<Ink>> {
        self.layout_blocks(text, spans, &[], &[], style, scale, max_width, align)
    }

    /// [`layout_aligned`](Self::layout_aligned) with per-paragraph [`Block`](crate::Block)
    /// indentation applied during line breaking, and room around each pill's label for its
    /// chip. A pill never wraps inside; one that starts a line starts its chip at the line's start.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn layout_blocks(
        &mut self,
        text: &str,
        spans: &[Span],
        blocks: &[crate::edit::Block],
        pills: &[PillChip],
        style: TextStyle,
        scale: f32,
        max_width: Option<f32>,
        align: Alignment,
    ) -> Rc<Layout<Ink>> {
        self.layout_blocks_inner(text, spans, blocks, pills, style, scale, max_width, align, false, true)
    }

    /// Secure display runs must not enter the shared layout cache or leave character
    /// scratch in the reusable shaping context. Layouts contain glyphs and byte ranges,
    /// not source strings; only the mounted node owns a secure run's layout.
    pub(crate) fn layout_for_spec(
        &mut self,
        spec: &crate::el::TextSpec,
        scale: f32,
        max_width: Option<f32>,
        align: Alignment,
    ) -> Rc<Layout<Ink>> {
        self.layout_blocks_inner(&spec.content, &spec.spans, &spec.blocks, &spec.pills, spec.style, scale, max_width, align, spec.wrap_anywhere, !spec.redacted)
    }
}
