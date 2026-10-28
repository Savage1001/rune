# Node storage inventory

Exploratory instrumentation, disabled unless `storage-probe` is enabled:

```sh
cargo run --profile perf -p rune --features storage-probe --example storage_probe
```

This creates a CPU-only runtime, with no window or renderer, and switches a
column between 500 and 10 rows. Each row contains a text element and a button.
It prints inline field sizes, occurrence counts and node-vector capacity.

At revision `796f786417c619658183619c88709e242e1fa68c`, with Taffy 0.15.1,
an optimized arm64 run found:

- Node: 2,992 bytes; El: 3,400 bytes.
- Visual inside Node: 960 bytes.
- Optional text specification inside Node: 616 bytes, present in 500 of 1,501 nodes.
- The selected optional motion fields: 788 bytes inside each node, absent in this fixture.
- Taffy Style: 552 bytes; Cache: 408 bytes; Layout: 92 bytes.
- Warmed node-vector capacity: 1,501, or 4,490,992 bytes.
- Following the 500→10→500 row swap, capacities were 1,501→128→88→2,816.

Field sizes overlap: Visual and text are already included in Node. They are not
additional retained heap to sum. The small frames still contain exiting nodes;
the example advances no animation time. Type sizes do not include pointee
allocations or allocator metadata and are not physical footprint measurements.

The probe identifies cold-capability storage as a compaction candidate. It does
not implement compaction, buffer reuse or a replacement Taffy adapter, and it
does not establish a timing improvement. A spare node vector would reduce
allocation traffic while retaining another generation of storage; any experiment
must measure both and exercise large-to-small transitions.
