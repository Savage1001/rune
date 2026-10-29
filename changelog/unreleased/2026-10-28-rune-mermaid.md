- **New crate `rune-mermaid`: native Mermaid diagrams in Rust.** Extracted from `rune-kit` so
  apps can render Mermaid diagrams (`rune_mermaid::render` with `DiagramStyle`) without pulling
  in `rune-kit` or `pulldown-cmark`, while `rune-kit`'s `mermaid` feature continues to draw
  closed ```` ```mermaid ```` fences inside `Markdown`. Adds `stateDiagram` / `stateDiagram-v2`
  (`[*]`, composite states, `<<choice>>`), theme-adapted node tints from `style` / `classDef` /
  `:::class`, quoted edge labels, and nested brackets inside unquoted node labels.
