//! Browser selection is per browser; daemon connections outlive session selection.
use axum::{
    Router,
    extract::{Form, Request, State},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use misa_client::daemons::{Daemon, Daemons};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tower::ServiceExt;
#[path = "lifecycle.rs"]
mod lifecycle;

struct Hub {
    connections: Arc<Daemons>,
    daemons: BTreeMap<String, Arc<Daemon>>,
    overviews: BTreeMap<String, super::overview::Feed>,
    updates: tokio::sync::watch::Sender<u64>,
    sessions: BTreeMap<String, Instance>,
}
struct Instance {
    remote: Arc<super::Remote>,
    used: Instant,
}
const MAX_INSTANCES: usize = 64;
const IDLE_LIFETIME: Duration = Duration::from_secs(30 * 60);
impl Hub {
    fn new(connections: Arc<Daemons>, daemons: BTreeMap<String, Arc<Daemon>>) -> Self {
        let updates = tokio::sync::watch::channel(0).0;
        let overviews = daemons.iter().map(|(id, daemon)| (id.clone(), super::overview::Feed::start(daemon.clone(), updates.clone()))).collect();
        Self { connections, daemons, overviews, updates, sessions: BTreeMap::new() }
    }
    fn summaries(&self) -> String {
        self.daemons.iter().map(|(id, _)| format!("<section><h2>{}</h2>{}</section>", super::escape(id), self.overviews.get(id).map(|feed|feed.html()).unwrap_or_default())).collect()
    }
    fn expire(&mut self) {
        self.sessions.retain(|_, instance| {
            Arc::strong_count(&instance.remote) > 1 || instance.used.elapsed() < IDLE_LIFETIME
        });
    }
}
type Shared = Arc<Mutex<Hub>>;

pub async fn serve(targets: &[String], address: std::net::SocketAddr) -> Result<(), String> {
    let mut targets = targets.to_vec();
    #[cfg(unix)]
    if targets.is_empty() {
        targets = misa_transport::local::discover().await?;
    }
    let identity =
        misa_transport::identity::load(&misa_transport::identity::client_path("misa-web")?)?;
    let endpoint = misa_transport::iroh::bind(Some(identity), true).await?;
    let connections = Arc::new(Daemons::new(
        endpoint,
        misa_proto::ClientInfo::new("misa-web", env!("CARGO_PKG_VERSION")),
    ));
    let mut daemons = BTreeMap::new();
    for target in targets {
        let (daemon, _) = connections
            .connect_target(&target)
            .await
            .map_err(|fault| fault.message)?;
        daemons.insert(daemon.identity().to_owned(), daemon);
    }
    let hub = Hub::new(connections, daemons);
    let hub = Arc::new(Mutex::new(hub));
    let weak = Arc::downgrade(&hub);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let Some(hub) = weak.upgrade() else {
                break;
            };
            hub.lock().await.expire();
        }
    });
    let app = Router::new()
        .route("/daemons", get(directory))
        .route("/daemons/events", get(events))
        .route("/overview.js", get(|| async { ([("content-type", "text/javascript")], include_str!("overview.js")) }))
        .route("/opening.js", get(|| async { ([("content-type", "text/javascript")], include_str!("opening.js")) }))
        .route("/recovery.js", get(|| async { ([("content-type", "text/javascript")], include_str!("recovery.js")) }))
        .route("/connect", post(connect))
        .route("/choose", post(choose))
        .route("/lifecycle", get(lifecycle::open).post(lifecycle::perform))
        .route("/archive", get(lifecycle::archive))
        .route("/disconnect", post(lifecycle::disconnect))
        .fallback(dispatch)
        .with_state(hub);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(
        "serving at http://{}",
        listener.local_addr().map_err(|e| e.to_string())?
    );
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}

async fn directory(State(hub): State<Shared>) -> Html<String> {
    let hub = hub.lock().await;
    let mut html = String::from(
        "<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width\"><title>Misa · Daemons</title><link rel=\"stylesheet\" href=\"/style.css\"></head><body><main><h1>Daemons</h1><form method=\"post\" action=\"/connect\"><label>Daemon address or pairing ticket <input name=\"target\" required></label><button>Connect</button></form>",
    );
    html.push_str("<p>Each daemon reports its own latest publication. Related session state carries its own freshness.</p><p id=\"overview-status\" role=\"status\"></p><div id=\"overview\">");
    html.push_str(&hub.summaries());
    html.push_str("</div>");
    html.push_str("<section><h2>Manage sessions</h2><form method=\"get\" action=\"/lifecycle\"><label>Daemon <select name=\"daemon\">");
    for id in hub.daemons.keys() { html.push_str(&format!("<option value=\"{}\">{}</option>", super::escape(id), super::escape(id))); }
    html.push_str("</select></label><select name=\"command\"><option value=\"daemon.session.create\">Create session</option><option value=\"daemon.session.resume\">Resume conversation</option><option value=\"daemon.session.close\">Close session owner</option></select><button>Prepare</button><button formaction=\"/archive\">Browse saved conversations</button></form></section>");
    for id in hub.daemons.keys() {
        html.push_str(&format!("<form method=\"post\" action=\"/disconnect\"><input type=\"hidden\" name=\"daemon\" value=\"{}\"><button>Disconnect {}</button></form>", super::escape(id), super::escape(id)));
    }
    if !hub.sessions.is_empty() {
        html.push_str("<section><h2>Retained presentations</h2><p>Close unused presentations to release their observations. Session work continues.</p>");
        for (key, instance) in &hub.sessions {
            let title = &instance.remote.session.title;
            html.push_str(&format!("<form method=\"post\" action=\"/view/{}/close\"><span>{} · {}</span> <button>Close presentation</button></form>", super::escape(key), super::escape(title), super::escape(key)));
        }
        html.push_str("</section>");
    }
    html.push_str("</main><script src=\"/opening.js\"></script><script src=\"/overview.js\"></script></body></html>");
    Html(html)
}

async fn events(State(hub): State<Shared>) -> Response {
    use axum::response::{Sse, sse::{Event, KeepAlive}};
    let changes = hub.lock().await.updates.subscribe();
    let stream = futures::stream::unfold((hub, changes, true), |(hub, mut changes, first)| async move {
        if !first && changes.changed().await.is_err() { return None; }
        let html = hub.lock().await.summaries();
        Some((Ok::<_, std::convert::Infallible>(Event::default().data(html)), (hub, changes, false)))
    });
    let mut response = Sse::new(stream).keep_alive(KeepAlive::default()).into_response();
    response.headers_mut().insert(axum::http::header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

async fn connect(
    State(hub): State<Shared>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Response {
    let Some(target) = form.get("target") else {
        return "A daemon address is required".into_response();
    };
    let connections = hub.lock().await.connections.clone();
    match connections.connect_target(target).await {
        Ok((daemon, _)) => {
            let mut hub = hub.lock().await;
            let id = daemon.identity().to_owned();
            if !hub.overviews.contains_key(&id) {
                let feed = super::overview::Feed::start(daemon.clone(), hub.updates.clone());
                hub.overviews.insert(id.clone(), feed);
            }
            hub.daemons.insert(id, daemon);
            hub.updates.send_modify(|version| *version = version.wrapping_add(1));
            Redirect::to("/daemons").into_response()
        }
        Err(error) => (axum::http::StatusCode::BAD_GATEWAY, error.message).into_response(),
    }
}

async fn choose(State(hub): State<Shared>, Form(form): Form<BTreeMap<String, String>>) -> Response {
    let (Some(id), Some(session), Some(incarnation)) = (form.get("daemon"), form.get("session"), form.get("incarnation")) else {
        return (axum::http::StatusCode::BAD_REQUEST, "Choose a current daemon and session from the overview").into_response();
    };
    let daemon = hub.lock().await.daemons.get(id).cloned();
    let Some(daemon) = daemon else {
        return "Unknown daemon".into_response();
    };
    let preferences = match super::presentations::initial(form.get("preferences").map(String::as_str)) {
        Ok(preferences) => preferences,
        Err(error) => return (axum::http::StatusCode::BAD_REQUEST, error).into_response(),
    };
    match super::remote::connect_with(&daemon, session, preferences).await {
        Ok(remote) => {
            if incarnation != &remote.interaction.interface.scope.incarnation {
                return (axum::http::StatusCode::CONFLICT, "Session changed; choose its current owner from the overview").into_response();
            }
            let key = remote.instance.clone();
            let mut hub = hub.lock().await;
            hub.expire();
            if hub.sessions.len() >= MAX_INSTANCES {
                return (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    "Close an existing presentation before opening another",
                )
                    .into_response();
            }
            hub.sessions.insert(
                key.clone(),
                Instance {
                    remote,
                    used: Instant::now(),
                },
            );
            Redirect::to(&format!("/view/{key}/")).into_response()
        }
        Err(error) => (axum::http::StatusCode::BAD_GATEWAY, error).into_response(),
    }
}

async fn dispatch(State(hub): State<Shared>, mut request: Request) -> Response {
    if request.uri().path() == "/style.css" {
        return ([("content-type", "text/css")], super::STYLE).into_response();
    }
    let path = request.uri().path().to_owned();
    let Some((key, suffix)) = path
        .strip_prefix("/view/")
        .and_then(|path| path.split_once('/'))
    else {
        return Redirect::to("/daemons").into_response();
    };
    if suffix == "close" && request.method() == axum::http::Method::POST {
        if let Some(instance) = hub.lock().await.sessions.remove(key) {
            instance.remote.close();
        }
        return Redirect::to("/daemons").into_response();
    }
    let remote = {
        let mut hub = hub.lock().await;
        hub.sessions.get_mut(key).map(|instance| {
            instance.used = Instant::now();
            instance.remote.clone()
        })
    };
    let Some(remote) = remote else {
        return (axum::http::StatusCode::GONE, [("cache-control", "no-store")], Html(format!("<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width\"><title>Presentation expired</title><link rel=\"stylesheet\" href=\"/style.css\"></head><body data-expired-view=\"{}\"><main><h1>Presentation expired</h1><p>This local view is no longer retained. Session work may still be running.</p><p><a href=\"/daemons\">Choose a current session</a></p><section id=\"recovery\" aria-label=\"Local draft recovery\"><p>Enable JavaScript to read any drafts retained in this browser.</p></section></main><script src=\"/recovery.js\"></script></body></html>", super::escape(key)))).into_response();
    };
    if suffix == "claim" && request.method() == axum::http::Method::POST {
        if remote.claimed.compare_exchange(false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire).is_ok() {
            return axum::Json(serde_json::json!({"url":format!("/view/{key}/")})).into_response();
        }
        let session = &remote.session;
        let preferences = remote.preferences.lock().unwrap().clone();
        let fork = match super::remote::connect_with(&remote.daemon, &session.id, preferences).await {
            Ok(fork) => fork,
            Err(error) => return (axum::http::StatusCode::BAD_GATEWAY, error).into_response(),
        };
        if fork.interaction.interface.scope != remote.interaction.interface.scope {
            return (axum::http::StatusCode::CONFLICT, "The session restarted; select its new incarnation from Daemons and sessions").into_response();
        }
        let mut hub = hub.lock().await;
        hub.expire();
        if hub.sessions.len() >= MAX_INSTANCES { return (axum::http::StatusCode::TOO_MANY_REQUESTS, "Close an existing presentation before opening another tab").into_response(); }
        let key = fork.instance.clone();
        hub.sessions.insert(key.clone(), Instance { remote: fork, used: Instant::now() });
        let memory = format!("{}:{}:{}", remote.daemon.identity(), remote.interaction.interface.scope.incarnation, key);
        return axum::Json(serde_json::json!({"url":format!("/view/{key}/"),"memory":memory})).into_response();
    }
    let query = request
        .uri()
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    *request.uri_mut() = format!("/{suffix}{query}")
        .parse()
        .expect("original URI suffix");
    match super::remote_router(remote).oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{StatusCode, header},
    };
    use misa_protocol::invocation::CommandOwner;

    #[tokio::test]
    async fn view_urls_isolate_tabs_and_active_streams_keep_their_instance_alive() {
        let directory = misa_daemon::directory::Directory::new("web-tabs").unwrap();
        for id in ["first", "second"] {
            directory
                .insert(misa_session::Runtime::start(
                    id,
                    id,
                    None,
                    Arc::new(misa_kernel::LocalKernel::new(
                        misa_kernel::ScriptedProvider::always("reply"),
                    )),
                    "scripted",
                    "scripted-1",
                    misa_value::Value::Null,
                ))
                .unwrap();
        }
        let server = misa_transport::iroh::bind(None, false).await.unwrap();
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let router = iroh::protocol::Router::builder(server.clone())
            .accept(
                misa_proto::scoped::ALPN,
                misa_transport::scoped_server::Handler {
                    daemon: server.id().to_string(),
                    scope: directory.scope(),
                    resolver: Arc::new(misa_daemon::directory::Routes(directory)),
                    admission: Arc::new(misa_transport::admission::Admission::open()),
                },
            )
            .spawn();
        let connections = Arc::new(Daemons::new(
            endpoint.clone(),
            misa_proto::ClientInfo::new("web-tabs", "1"),
        ));
        let daemon = connections.connect(server.addr()).await.unwrap();
        let id = daemon.identity().to_owned();
        let mut watch = daemon.watch();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !matches!(
                daemon.sessions().unwrap().status,
                misa_protocol::observation::Status::Current
            ) {
                watch.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let hub = Arc::new(Mutex::new(Hub::new(connections, BTreeMap::from([(id.clone(), daemon)]))));
        let mut paths = Vec::new();
        for session in ["first", "second"] {
            let incarnation = hub.lock().await.daemons[&id].sessions().unwrap().sessions.into_iter().find(|entry| entry.id == session).unwrap().incarnation;
            let response = choose(
                State(hub.clone()),
                Form(BTreeMap::from([
                    ("daemon".into(), id.clone()),
                    ("session".into(), session.into()),
                    ("incarnation".into(), incarnation),
                    ("preferences".into(), "{\"status\":\"hide\"}".into()),
                ])),
            )
            .await;
            assert_eq!(response.status(), StatusCode::SEE_OTHER);
            assert!(!response.headers().contains_key(header::SET_COOKIE));
            paths.push(
                response.headers()[header::LOCATION]
                    .to_str()
                    .unwrap()
                    .to_owned(),
            );
        }
        assert_ne!(paths[0], paths[1]);
        for instance in hub.lock().await.sessions.values() {
            assert_eq!(instance.remote.preferences.lock().unwrap().0.get("status"), Some(&misa_client::composition::Choice::Hidden));
        }
        let mut claimed_paths = Vec::new();
        for _ in 0..2 {
            let response = dispatch(State(hub.clone()), Request::builder().method("POST")
                .uri(format!("{}claim", paths[0])).body(Body::empty()).unwrap()).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
            let claim: serde_json::Value = serde_json::from_slice(&body).unwrap();
            claimed_paths.push(claim["url"].as_str().unwrap().to_owned());
            if claimed_paths.len() == 2 { assert!(claim["memory"].as_str().unwrap().ends_with(claimed_paths[1].trim_end_matches('/').rsplit('/').next().unwrap())); }
        }
        assert_eq!(claimed_paths[0], paths[0]);
        for instance in hub.lock().await.sessions.values() {
            assert_eq!(instance.remote.preferences.lock().unwrap().0.get("status"), Some(&misa_client::composition::Choice::Hidden), "forks preserve preferences before observing content");
        }
        assert_ne!(claimed_paths[1], paths[0], "copied URL gets an independent presentation");
        {
            let hub = hub.lock().await;
            let key = |path: &str| path.trim_end_matches('/').rsplit('/').next().unwrap().to_owned();
            let first = &hub.sessions[&key(&claimed_paths[0])].remote;
            let copied = &hub.sessions[&key(&claimed_paths[1])].remote;
            assert!(!Arc::ptr_eq(first, copied));
            assert_eq!(first.interaction.interface.scope, copied.interaction.interface.scope);
            assert!(!Arc::ptr_eq(&first.pending, &copied.pending));
        }
        for (path, title) in paths.iter().zip(["first", "second"]) {
            let response = dispatch(
                State(hub.clone()),
                Request::builder()
                    .uri(path)
                    .header(header::COOKIE, "misa-session=wrong-tab")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let html = std::str::from_utf8(&body).unwrap();
            assert!(html.contains(&format!("<title>{title}</title>")), "{html}");
            assert!(html.contains("action=\"./close\""));
            assert!(html.contains("src=\"./app.js\""));
        }
        let events = dispatch(
            State(hub.clone()),
            Request::builder()
                .uri(format!("{}events", paths[0]))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(events.status(), StatusCode::OK);
        {
            let mut hub = hub.lock().await;
            for instance in hub.sessions.values_mut() {
                instance.used = Instant::now() - IDLE_LIFETIME - Duration::from_secs(1);
            }
            hub.expire();
            assert_eq!(
                hub.sessions.len(),
                1,
                "idle tab expires but SSE retains its own instance"
            );
        }
        let closed = dispatch(
            State(hub.clone()),
            Request::builder().method("POST").uri(format!("{}close", paths[0]))
                .body(Body::empty()).unwrap(),
        ).await;
        assert_eq!(closed.status(), StatusCode::SEE_OTHER);
        let body = tokio::time::timeout(Duration::from_secs(2), axum::body::to_bytes(events.into_body(), 1024 * 1024)).await
            .expect("closing a presentation must end its existing SSE lease").unwrap();
        assert!(std::str::from_utf8(&body).unwrap().contains("Presentation closed; session work continues"));
        hub.lock().await.expire();
        assert!(hub.lock().await.sessions.is_empty());
        let expired = dispatch(State(hub.clone()), Request::builder().uri(&paths[0]).body(Body::empty()).unwrap()).await;
        assert_eq!(expired.status(), StatusCode::GONE);
        let html = axum::body::to_bytes(expired.into_body(), 65536).await.unwrap();
        assert!(std::str::from_utf8(&html).unwrap().contains("data-expired-view="));
        drop(hub);
        endpoint.close().await;
        router.shutdown().await.unwrap();
    }
}
