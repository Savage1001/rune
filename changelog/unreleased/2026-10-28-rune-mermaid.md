- **New crate `rune-mermaid`: native Mermaid diagrams in Rust.** Extracted from `rune-kit` so
  apps can render Mermaid diagrams (`rune_mermaid::render` with `DiagramStyle`) without pulling
  in `rune-kit` or `pulldown-cmark`, while `rune-kit`'s `mermaid` feature continues to draw
  closed ```` ```mermaid ```` fences inside `Markdown`. Supports flowcharts (including v11
  `@{ shape: … }` nodes, subgraph-endpoint edges, quoted edge labels, nested brackets,
  `style`/`classDef` node tints and `linkStyle` edge tints), sequence diagrams (with `box`
  participant groups and `rect` bands), `stateDiagram` / `stateDiagram-v2`, `classDiagram`, and
  `title` headers; flowcharts, state diagrams and class diagrams share one layered renderer.
