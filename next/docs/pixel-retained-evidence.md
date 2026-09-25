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
The clock excludes App construction, scene layout, PNG encoding, Vulkan window presentation,
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
image ops still copy/upload pixels each frame. Pixel-width text layout remains to be addressed.

## Headless Vulkan baseline

`misa-skia-testbed --bench-gpu` drives the same backend-neutral App and Ganesh painter used
by the native window, but renders to an offscreen Vulkan target and synchronously reads back
RGBA pixels. On lavapipe at 800×600, six repeated frames passed the pixel-determinism oracle;
the fixture includes one decoded image. Release results below are median/best in **ms** over
12 samples per phase. App construction, PNG encoding, native swapchain presentation, and
network delivery are excluded; the GPU column **includes synchronization and readback**.

| Owners | Phase         | App frame/layout | Vulkan render + readback |
| -----: | ------------- | ---------------: | -----------------------: |
|     10 | cold          |    0.042 / 0.040 |            0.765 / 0.688 |
|     10 | unchanged     |    0.017 / 0.016 |            1.025 / 0.888 |
|     10 | replace owner |    0.041 / 0.039 |            1.017 / 0.902 |
|  1,000 | cold          |    1.961 / 1.915 |            2.084 / 1.944 |
|  1,000 | unchanged     |    0.042 / 0.041 |            2.022 / 1.935 |
|  1,000 | replace owner |    2.156 / 2.026 |            2.057 / 1.921 |

The offscreen GPU results do **not** establish a native-window latency or a speedup over the
CPU reference: readback and swapchain presentation have different costs, and these values
are measured on a software Vulkan driver. Measure the native frame-to-present path before
claiming a user-visible win.
