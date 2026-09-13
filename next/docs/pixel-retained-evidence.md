# Retained pixel presentation

The pixel connection forwards validated snapshots, operations and stream updates. It no longer
materializes `ClientView::rendered()` or scans every image reference on a stream append. The app
holds an indexed tree and separate live streams. Each identified node retains its local paint
operations; an update invalidates the changed node and its ancestors. Skia positions shared scene
groups without reconstructing their text, tables, controls or paint operations.

The measurement counts calls to semantic node layout, not elapsed time. Concurrent workspace builds
make timing comparisons unsuitable for this run. Before the change, an unchanged frame performed
12 layouts with 10 transcript owners and 1,002 with 1,000 owners. Six unchanged PNG renders at each
size produced one distinct output.

| Operation                    | 10 owners | 1,000 owners |
| ---------------------------- | --------: | -----------: |
| Unchanged retained frame     | 0 layouts |    0 layouts |
| Append to one live stream    | 4 layouts |    4 layouts |
| Replace one transcript owner | 3 layouts |    3 layouts |

The stream count includes the root, transcript, stream group and changed stream. Replacement
includes the root, transcript and replaced owner. Tests also assert that every untouched owner's
paint group retains its `Arc` identity. Retained output is byte-identical to a cold raster rebuild
after stream append, replacement, insertion, removal and stream completion. Editing a form rebuilds
the root and that form while retaining the transcript scene.

This is a reduction in semantic layout and scene generation. Hit/text metadata is still collected
for selection, and the window still raster-repaints; these counts do not claim constant total frame
cost or a measured frame-time speedup. The cache retains paint groups and interaction metadata in
memory in exchange for avoiding repeated layout. Resize invalidates the layout cache.

Validation uses `cargo test -p misa-skia` and the native Xvfb keyboard, clipboard and disclosure test
at `crates/misa-skia/tests/window.sh`, in the artifact-derived graphics shell.
