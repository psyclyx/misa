//! Open a pixel window, or export a session view as PNG.
//!
//! ```sh
//! # a session, live
//! misa-skia --ticket misa:<endpoint id>:<session> --out frame.png
//! # a view that was saved, with no session at all
//! misa-skia --view frame.json --out frame.png
//! # keep rendering as it changes
//! misa-skia --ticket misa:<endpoint id>:<session> --out frame.png --every-ms 500
//! ```
//!
//! The file is the useful half of a pixel frontend: it can be checked in a test, produced
//! on a machine with no display, and diffed against last week's. A window is a second
//! consumer of the same scene and needs nothing new from a session.

use std::collections::BTreeMap;
use std::time::Duration;

use misa_proto::observation::Selection;
use misa_proto::view::Node;
use misa_protocol::observation::{MemberState, Status};

fn usage() -> String {
    "usage: misa-skia [--start-local] [--ticket misa:<endpoint id>:<session> | --view view.json] [--window] [--out frame.png] [--every-ms N] [--columns N]"
        .into()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut window = false;
    let mut start_local = false;
    let mut ticket = None;
    let mut view_file = None;
    let mut out = None;
    let mut every_ms: Option<u64> = None;
    let mut columns = 100u32;
    let mut rows = 40u32;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--window" => window = true,
            "--start-local" => start_local = true,
            "--ticket" | "-t" => ticket = Some(arguments.next().ok_or("--ticket needs a value")?),
            "--view" | "-v" => view_file = Some(arguments.next().ok_or("--view needs a value")?),
            "--out" | "-o" => out = arguments.next(),
            "--every-ms" => every_ms = arguments.next().and_then(|value| value.parse().ok()),
            "--columns" => {
                columns = arguments
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(columns)
            }
            "--rows" => {
                rows = arguments
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(rows)
            }
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }
    if ticket.is_none() && view_file.is_none() && out.is_some() && !window {
        return Err(usage().into());
    }
    if start_local {
        if !cold_start_eligible(
            window || out.is_none(),
            ticket.is_some(),
            view_file.is_some(),
        ) {
            return Err("--start-local requires a live window without --ticket or --view".into());
        }
        #[cfg(unix)]
        misa_local_start::discover_or_start().await?;
        #[cfg(not(unix))]
        return Err("--start-local requires Unix".into());
    }

    let mut first: Option<Node> = match &view_file {
        Some(path) => {
            let text = std::fs::read_to_string(path)?;
            Some(serde_json::from_str(&text)?)
        }
        None => None,
    };

    if window || out.is_none() {
        let view = first.take().unwrap_or_else(|| Node::section("connecting"));
        let live = ticket.is_some() || view_file.is_none();
        misa_skia::window::run(view, ticket, out, live)?;
        return Ok(());
    }
    let out = out.expect("PNG output selected");
    let metrics = misa_skia_paint::text_metrics()
        .map_err(|error| format!("Cannot load Skia text metrics: {error}"))?;
    let mut renderer = misa_skia_vulkan::Renderer::new()?;

    if let Some(text) = first.take() {
        write_frame(&mut renderer, &text, columns, rows, &out, metrics.clone())?;
        println!("wrote {out}");
        if every_ms.is_none() {
            return Ok(());
        }
    }

    let Some(ticket) = ticket else {
        return Ok(());
    };
    // A ticket, or a pairing string: whatever the daemon printed or the QR said.
    let (parsed, _) = misa_proto::Pairing::given(&ticket)?;
    let identity =
        misa_transport::identity::load(&misa_transport::identity::client_path("misa-skia")?)?;
    let endpoint = misa_transport::iroh::bind(
        Some(identity),
        !misa_transport::iroh::names_only_this_machine(&parsed.node),
    )
    .await?;
    let info = misa_proto::ClientInfo::new("misa-skia", env!("CARGO_PKG_VERSION"));
    let daemons = misa_client::daemons::Daemons::new(endpoint, info);
    let (daemon, hint) = daemons
        .connect_target(&ticket)
        .await
        .map_err(|fault| fault.message)?;
    let mut directory = daemon.watch();
    let scope = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let snapshot = daemon.sessions().map_err(|fault| fault.message)?;
            if matches!(snapshot.status, Status::Current) {
                let selected =
                    match &hint {
                        Some(id) => snapshot.sessions.iter().find(|entry| &entry.id == id),
                        None if snapshot.sessions.len() == 1 => snapshot.sessions.first(),
                        None => return Err(
                            "Choose a session in the ticket when the daemon has several sessions"
                                .to_owned(),
                        ),
                    };
                return selected
                    .map(|entry| entry.scope())
                    .ok_or_else(|| "The selected session is no longer available".to_owned());
            }
            if let Status::Closed(fault) = snapshot.status {
                return Err(fault.message);
            }
            directory
                .changed()
                .await
                .map_err(|_| "Daemon directory closed".to_owned())?;
        }
    })
    .await
    .map_err(|_| "Timed out reading daemon sessions")??;
    let interface = misa_client::interface::Interface::load(&daemon.client, scope.clone())
        .await
        .map_err(|fault| fault.message)?;
    let member = interface
        .presentation("conversation", &[])
        .map_err(|fault| fault.message)?;
    let mut observation = daemon
        .client
        .observe(
            Selection {
                scope,
                members: BTreeMap::from([("conversation".into(), member)]),
            },
            None,
        )
        .await
        .map_err(|fault| fault.message)?;
    let mut painted = None;
    loop {
        let frame = observation
            .inspect(|replica, notice| {
                if let Status::Closed(fault) = replica.status() {
                    return Err(fault.message.clone());
                }
                let Some(notice) = notice else {
                    return Ok(None);
                };
                if painted == Some(notice.sequence) || !matches!(replica.status(), Status::Current)
                {
                    return Ok(None);
                }
                let Some(MemberState::Document(document)) = replica
                    .current()
                    .and_then(|members| members.get("conversation"))
                else {
                    return Ok(None);
                };
                Ok(Some((
                    notice.sequence,
                    misa_client::document::rendered(document),
                )))
            })
            .ok_or("Presentation observation closed")??;
        if let Some((sequence, view)) = frame {
            write_frame(&mut renderer, &view, columns, rows, &out, metrics.clone())?;
            painted = Some(sequence);
            println!("wrote {out}");
            if every_ms.is_none() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(every_ms.unwrap().max(16))).await;
        }
        observation.changed().await.map_err(|fault| fault.message)?;
    }
}

fn cold_start_eligible(live_window: bool, ticket: bool, view: bool) -> bool {
    live_window && !ticket && !view
}

#[cfg(test)]
mod cold_start_tests {
    use super::cold_start_eligible;
    #[test]
    fn excludes_explicit_and_offline_modes() {
        assert!(cold_start_eligible(true, false, false));
        for (window, ticket, view) in [
            (false, false, false),
            (true, true, false),
            (true, false, true),
        ] {
            assert!(!cold_start_eligible(window, ticket, view));
        }
    }
}

/// Paint a scene to a file.
fn write_frame(
    renderer: &mut misa_skia_vulkan::Renderer,
    view: &Node,
    columns: u32,
    rows: u32,
    out: &str,
    metrics: std::sync::Arc<dyn misa_skia_ui::TextMetrics>,
) -> Result<(), Box<dyn std::error::Error>> {
    let scene = snapshot_scene(view, columns, rows, metrics);
    let pixels = renderer.render(&scene, misa_style::Color::Rgb(20, 22, 26))?;
    let mut png = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut png, image::ImageFormat::Png)?;
    std::fs::write(out, png.into_inner())?;
    Ok(())
}

fn snapshot_scene(
    view: &Node,
    columns: u32,
    rows: u32,
    metrics: std::sync::Arc<dyn misa_skia_ui::TextMetrics>,
) -> misa_skia_ui::Scene {
    // Size the offline frame using the resolved paint typeface's measurements.
    let font_size = 15.0;
    let margin = 24.0;
    let width = (margin * 2.0 + columns as f32 * metrics.measure("M", font_size)).ceil() as u32;
    let height =
        (margin * 2.0 + rows as f32 * metrics.line_metrics(font_size).line_height).ceil() as u32;
    let mut app = misa_skia::app::App::new(view.clone(), metrics);
    app.pin_to_top();
    app.frame_at(width, height, Duration::ZERO)
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use misa_proto::view::Span;
    use misa_skia_ui::Op;

    #[test]
    fn snapshot_keeps_first_glyphs_visible_at_one_and_multiple_rows() {
        let metrics = misa_skia_paint::text_metrics().expect("Skia typeface");
        let line_height = metrics.line_metrics(15.0).line_height;
        let mut renderer =
            misa_skia_vulkan::Renderer::new().expect("Vulkan readback (lavapipe is fine)");
        let view = Node::section("root").children(
            (0..8).map(|i| Node::text("text", [Span::plain(format!("VISIBLE line {i}"))])),
        );
        for rows in [1, 4] {
            let scene = snapshot_scene(&view, 40, rows, metrics.clone());
            assert!(
                matches!(scene.ops.first(), Some(Op::Group { y, .. }) if (*y - 20.0).abs() < 0.01),
                "snapshot must start at the top for {rows} rows"
            );
            let pixels = renderer
                .render(&scene, misa_style::Color::Rgb(20, 22, 26))
                .expect("GPU render/readback");
            let (width, height) = pixels.dimensions();
            assert_eq!((width, height), (scene.width as u32, scene.height as u32));
            // Only the first row's glyphs can land in this band: the later rows are
            // separated by their line height and the node's trailing space.
            let ink = (20..height.min((20.0 + line_height) as u32)).any(|y| {
                (20..width.min(180)).any(|x| pixels.get_pixel(x, y).0[..3] != [20, 22, 26])
            });
            assert!(ink, "first glyphs missing from the {rows}-row PNG viewport");
        }

        // The same App without the offline choice still follows a long view.
        let mut window = misa_skia::app::App::new(view, metrics);
        let followed = window.frame_at(380, 70, Duration::ZERO);
        assert!(matches!(followed.ops.first(), Some(Op::Group { y, .. }) if *y < 0.0));
    }
}
