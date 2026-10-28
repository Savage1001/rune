//! Representation inventory before choosing a node-compaction experiment.
use super::*;
use std::mem::{size_of, size_of_val};

impl<S: 'static> Runtime<S> {
    /// Print field sizes and occurrence counts for the opt-in storage experiment.
    #[doc(hidden)]
    pub fn print_storage_inventory(&self) {
    let rt = self;
    if rt.nodes.is_empty() { return; }
    let n = &rt.nodes[0];
    println!("METRIC probe=node-storage nodes={} capacity={} node_bytes={} vector_bytes={} el_bytes={} visual_bytes={}",
        rt.nodes.len(), rt.nodes.capacity(), size_of::<Node>(), rt.nodes.capacity() * size_of::<Node>(), size_of::<El>(), size_of_val(&n.vis));
    // Size of each inline optional field, plus its usage in the same fixture.
    macro_rules! field {
        ($name:ident) => {
            println!("METRIC probe=node-field field={} bytes={} present={} nodes={}",
                stringify!($name), size_of_val(&n.$name), rt.nodes.iter().filter(|n| n.$name.is_some()).count(), rt.nodes.len());
        };
    }
    field!(looping);
    field!(text);
    field!(opacity_motion);
    field!(lead);
    field!(transition);
    field!(matched);
    field!(growth);
    field!(input);
    field!(modal);
    macro_rules! visual {
        ($name:ident) => {
            println!("METRIC probe=visual-field field={} bytes={} present={} nodes={}",
                stringify!($name), size_of_val(&n.vis.$name), rt.nodes.iter().filter(|n| n.vis.$name.is_some()).count(), rt.nodes.len());
        };
    }
    visual!(blur_motion);
    visual!(hover_motion);
    visual!(hover_within);
    visual!(offset_motion);
    visual!(child_exit);
    visual!(radial);
    println!("METRIC probe=layout-types style_bytes={} cache_bytes={} layout_bytes={}",
        size_of::<taffy::Style>(), size_of::<taffy::Cache>(), size_of::<taffy::Layout>());
    }
}
