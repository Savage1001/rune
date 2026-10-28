- **A typed line stays clear of a scroll viewport's faded edges.** After an edit, the focused
  field's caret now stops `El::scroll_margin` short of its viewport's edge, as
  `Tx::scroll_into_view` does and as CSS `scroll-padding` does for a textarea, so the line being
  typed never sits under a `scroll_edge_fade`. The content's own ends still stop flush.
