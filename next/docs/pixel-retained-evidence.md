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
for selection, and the window still repaints the full scene through Ganesh/Vulkan; these counts
do not claim constant total frame cost or a measured frame-time speedup. The cache retains paint groups and interaction metadata in
memory in exchange for avoiding repeated layout. Resize invalidates the layout cache.

The primary window-independent validation is the Vulkan headless driver:
`cargo test -p misa-skia-vulkan -p misa-skia-testbed` (with the pinned Skia Vulkan
archive, Vulkan loader, and software ICD). It sends normalized input and fake time
through the same UI path as native hosts, then reads back actual Ganesh pixels.
The native window test at `crates/misa-skia/tests/window.sh` is secondary platform
adapter coverage.

## Offline raster baseline and font-resolution experiment

The client-free `misa-skia-testbed --bench-ab` paints the _same_ retained warm scene at
800×600 through the former **CPU raster reference** path, with and without a successful
typeface cached across frames. This is a release-build, same-binary, interleaved ABBA/BAAB comparison:
six blocks, 12 samples per mode per fixture. Uncached pixels are the reference; six unchanged
frames, the changed-owner scene, the profiled path, and the production path match pixel-for-pixel.
The clock excludes DocumentUi construction, scene layout, PNG encoding, Vulkan window presentation,
and network delivery. Median/best raster times in **ms**, measured after splitting the UI crate:

| Fixture      |        Cached |      Uncached |
| ------------ | ------------: | ------------: |
| 10 owners    | 0.440 / 0.432 | 7.759 / 7.632 |
| 1,000 owners | 1.716 / 1.681 | 9.125 / 9.043 |

The previously unmeasured cost was chiefly resolving the system monospace typeface on _every_
raster (uncached font-resolution median 6.3 ms); draw work also decreased in this experiment.
The cache retains only a successful resolution; a missing font can be retried. The first paint
still pays font discovery. These historical CPU measurements are **not Vulkan frame-time
measurements** and say nothing about end-to-end window latency or font changes after the first
successful resolution. Native windows and headless UI tests now paint with the same Ganesh
Vulkan backend; screenshots use explicit GPU readback. Skia still repaints the full scene and
image ops still copy/upload pixels each frame. Text rows, wrapping, caret positions,
selection and hits now share measured glyph advances from the same cached typeface the
painter uses; narrow rows are clipped on both raster and Vulkan canvases.

## Headless Vulkan baseline

The following baseline predates measured text layout; it is retained to show the
cost of the correctness cutover, not as an equivalent-scene speed comparison.

`misa-skia-testbed --bench-gpu` drives the same backend-neutral DocumentUi and Ganesh painter used
by the native window, but renders to an offscreen Vulkan target and synchronously reads back
RGBA pixels. On lavapipe at 800×600, six repeated frames passed the pixel-determinism oracle;
the fixture includes one decoded image. Release results below are median/best in **ms** over
12 samples per phase. DocumentUi construction, PNG encoding, native swapchain presentation, and
network delivery are excluded; the GPU column **includes synchronization and readback**.

| Owners | Phase         | DocumentUi frame/layout | Vulkan render + readback |
| -----: | ------------- | ----------------------: | -----------------------: |
|     10 | cold          |           0.042 / 0.040 |            0.765 / 0.688 |
|     10 | unchanged     |           0.017 / 0.016 |            1.025 / 0.888 |
|     10 | replace owner |           0.041 / 0.039 |            1.017 / 0.902 |
|  1,000 | cold          |           1.961 / 1.915 |            2.084 / 1.944 |
|  1,000 | unchanged     |           0.042 / 0.041 |            2.022 / 1.935 |
|  1,000 | replace owner |           2.156 / 2.026 |            2.057 / 1.921 |

With measured text and shared retained row geometry, a subsequent release run of
`--bench-gpu` on the same lavapipe device at 800×600 (median/best ms, 12 samples
per phase) yielded:

| Owners | Phase         | DocumentUi frame/layout | Vulkan render + readback |
| -----: | ------------- | ----------------------: | -----------------------: |
|     10 | cold          |           0.077 / 0.074 |            0.845 / 0.798 |
|     10 | unchanged     |           0.017 / 0.016 |            1.041 / 0.956 |
|     10 | replace owner |           0.080 / 0.076 |            1.111 / 1.014 |
|  1,000 | cold          |           4.993 / 4.833 |            2.257 / 2.126 |
|  1,000 | unchanged     |           0.035 / 0.033 |            2.226 / 2.077 |
|  1,000 | replace owner |           5.277 / 5.038 |            2.307 / 2.115 |

After the retained scene and field-widget split, a further release run of the
same offscreen harness on lavapipe at 800×600 yielded (median/best ms, 12 samples
per phase; stable same-device pixel readbacks and a visible edit verified):

| Owners | Phase         | DocumentUi frame/layout | Vulkan render + readback |
| -----: | ------------- | ----------------------: | -----------------------: |
|     10 | cold          |           0.075 / 0.073 |            0.880 / 0.769 |
|     10 | unchanged     |           0.018 / 0.017 |            1.054 / 0.948 |
|     10 | replace owner |           0.030 / 0.029 |            1.102 / 0.974 |
|  1,000 | cold          |           4.759 / 4.467 |            2.322 / 2.120 |
|  1,000 | unchanged     |           0.036 / 0.034 |            2.255 / 2.066 |
|  1,000 | replace owner |           0.299 / 0.229 |            2.271 / 2.100 |

With exact tail/anchor placement, a later release run on the same lavapipe
setup yielded (median/best ms, 12 samples per phase):

| Owners | Phase         | DocumentUi frame/layout | Vulkan render + readback |
| -----: | ------------- | ----------------------: | -----------------------: |
|     10 | cold          |           0.079 / 0.077 |            0.783 / 0.742 |
|     10 | unchanged     |           0.025 / 0.025 |            0.776 / 0.738 |
|     10 | replace owner |           0.035 / 0.033 |            0.796 / 0.734 |
|  1,000 | cold          |           0.130 / 0.122 |            1.219 / 1.066 |
|  1,000 | unchanged     |           0.031 / 0.029 |            1.131 / 0.983 |
|  1,000 | replace owner |           0.042 / 0.039 |            1.110 / 0.984 |

The 10,000-owner invariant test also bounds cold tail, backward-wheel, resize,
and disclosure work to fewer than 32 measured/placed owners. These are separate
code revisions with different scene composition and single-session samples, not
an interleaved A/B comparison or native-window latency. Readback and swapchain
presentation have different costs. Measure native frame-to-present before
claiming a user-visible win.

Background prewarm now exists and is documented in `docs/background-layout.md`:
one worker per presented document measures owners away from the frame, keeps
exact heights in a lazy index, retains display lists only near the viewport in
a bounded cache, and can advance all of it without presenting another frame.
Two earlier prototypes were **reverted**, not shipped. A height-only worker
stored exact offscreen heights but remeasured them on first visible placement,
so it added work without avoiding the expensive font layout. A later worker
prewarmed complete retained scenes and reused nearby geometry, but a full
history scan retained unbounded offscreen scene data and woke the native GPU
painter for each batch. A two-job channel bounded in-flight work, not retained
memory or total presentations. The shipped design keeps the exact height index
separate from a bounded near-viewport scene cache and advances indexing on a
wake that paints nothing.
