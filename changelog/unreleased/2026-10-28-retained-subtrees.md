- **Unchanged `Cx::memo` subtrees keep their mounted nodes across commits.** A `memo` hit used to
  clone its shared `El` tree and flatten, re-register and re-apply every node in it on each commit.
  Mounted `memo` subtrees whose identity, parent path and inherited layout context have not changed
  now splice their retained `Node` slice in place, keep their Taffy nodes, `NodeId`s and cached text
  layouts unless the available width changed, and skip redundant scene property updates (`Runtime::set_subtree_retention`,
  `SubtreeRetention::{FullRebuild, BuilderMemo, RetainedMounted}`, `Runtime::retained_subtree_stats`).
