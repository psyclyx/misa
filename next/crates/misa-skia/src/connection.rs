//! One network owner for the pixel client; window events never block on IO.
use crate::app::Command;
use misa_proto::view::{Kind, Node};
use misa_proto::{Query, SessionMsg, SubId};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use winit::event_loop::EventLoopProxy;

#[derive(Clone, Debug)]
pub enum Update {
    View(Node),
    Info(misa_proto::wire::SessionInfo),
    Image {
        hash: String,
        image: Arc<image::RgbaImage>,
    },
    Notice(String),
}

pub fn start(ticket: String, proxy: EventLoopProxy<Update>) -> tokio::sync::mpsc::Sender<Command> {
    let (send, receive) = tokio::sync::mpsc::channel(32);
    tokio::spawn(async move {
        if let Err(error) = run(&ticket, &proxy, receive).await {
            let _ = proxy.send_event(Update::Notice(error));
        }
    });
    send
}
async fn run(
    ticket: &str,
    proxy: &EventLoopProxy<Update>,
    mut outgoing: tokio::sync::mpsc::Receiver<Command>,
) -> Result<(), String> {
    let (ticket, code) = misa_proto::Pairing::given(ticket).map_err(|error| error.to_string())?;
    let endpoint = misa_net::iroh::bind_for(&ticket.node).await?;
    let address = misa_net::iroh::address_of(&ticket.node)?;
    if let Some(code) = code {
        misa_net::iroh::Client::pair(&endpoint, address.clone(), &code, "the pixel frontend")
            .await?;
    }
    let blobs = misa_net::blob::Store::new(endpoint.clone(), address.clone());
    let mut client = misa_net::iroh::Client::connect(
        &endpoint,
        address,
        misa_proto::ClientInfo::new("misa-skia", env!("CARGO_PKG_VERSION")),
        &ticket.session,
    )
    .await?;
    if let Some(info) = client.session() {
        let _ = proxy.send_event(Update::Info(info.clone()));
    }
    client
        .subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY))
        .await?;
    let mut current = misa_proto::sync::ClientView::default();
    let mut fetched = BTreeSet::new();
    let mut saves = BTreeMap::<u64, (String, tokio::time::Instant)>::new();
    let mut next = 1u64;
    let mut expiry = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {
            _=expiry.tick()=>{
                let expired: Vec<_> = saves.iter().filter(|(_,(_,started))| started.elapsed() >= std::time::Duration::from_secs(30)).map(|(id,_)| *id).collect();
                for id in expired {saves.remove(&id);let _=proxy.send_event(Update::Notice("Saving timed out; choose Save attachment to try again".into()));}
            },
            command=outgoing.recv()=>match command {
                Some(Command::Intent(intent))=>{client.intent(next,intent).await?;next+=1;}
                Some(Command::Save {node,destination})=>{
                    saves.insert(next,(destination,tokio::time::Instant::now()));
                    client.intent(next,misa_proto::wire::Intent::Action {node,action:"attachment.save".into(),args:misa_value::Value::Null,fields:vec![]}).await?;next+=1;
                }
                Some(Command::Copy(_))=>{}
                None=>return Ok(()),
            },
            message=client.next()=> {
                let Some(message)=message? else {return Err("The session disconnected".into());};
                match current.receive(&message) {
                    Ok(true)=>if let Some(view)=current.rendered() {
                        let _=proxy.send_event(Update::View(view.clone()));
                    for hash in images(&view) {
                        if !fetched.insert(hash.clone()) {continue;}
                        match blobs.get(&hash).await {
                            Ok(Some(blob))=>match image::load_from_memory(&blob.bytes) {
                                Ok(image)=>{let _=proxy.send_event(Update::Image {hash,image:Arc::new(image.into_rgba8())});}
                                Err(error)=>{let _=proxy.send_event(Update::Notice(format!("Could not decode attachment: {error}")));}
                            }
                            Ok(None)=>{let _=proxy.send_event(Update::Notice("An attachment is no longer available".into()));}
                            Err(error)=>{let _=proxy.send_event(Update::Notice(error));}
                        }
                    }
                    },
                    Err(error)=>{
                        let _=proxy.send_event(Update::Notice(format!("Refreshing session: {error}")));
                        client.subscribe(SubId(1),Query::new(misa_proto::VIEW_QUERY)).await?;
                    },
                    Ok(false)=>{}
                }
                match message {
                SessionMsg::Download {id,download}=>if let Some((destination,_))=saves.remove(&id) {
                    let result=async {
                        let reference=download.blob.ok_or(download.error)?;
                        let blob=blobs.get(&reference.hash).await?.ok_or("The attachment is no longer available")?;
                        if blob.bytes.len() as u64!=reference.len {return Err("The attachment length changed".into());}
                        write_new(&destination,&blob.bytes)?;
                        Ok::<_,String>(format!("Saved {destination}"))
                    }.await;
                    let _=proxy.send_event(Update::Notice(result.unwrap_or_else(|error|error)));
                },
                SessionMsg::Event {event,..}=>match event {
                    misa_proto::SessionEvent::Notice {text,..}|misa_proto::SessionEvent::Status {text}=>{let _=proxy.send_event(Update::Notice(text));}
                    _=>{}
                },
                SessionMsg::Fault {id,fault}=>{if let Some(id)=id{saves.remove(&id);}let _=proxy.send_event(Update::Notice(fault.message));}
                _=>{}
                }
            }
        }
    }
}
fn images(node: &Node) -> Vec<String> {
    let mut hashes = vec![];
    if let Kind::Image { blob, .. } = &node.kind {
        if blob
            .media
            .as_deref()
            .is_some_and(|media| media.starts_with("image/"))
        {
            hashes.push(blob.hash.clone());
        }
    }
    for child in &node.children {
        hashes.extend(images(child));
    }
    if let Kind::List { items, .. } = &node.kind {
        for child in items.iter().flatten() {
            hashes.extend(images(child));
        }
    }
    hashes
}
pub fn write_new(path: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Could not create {path}: {error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Could not finish {path}: {error}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_received_file_is_findable_and_never_overwrites_an_existing_file() {
        let path = std::env::temp_dir().join(format!("misa-pixel-save-{}", std::process::id()));
        let path = path.to_str().unwrap();
        let _ = std::fs::remove_file(path);
        super::write_new(path, b"received attachment").unwrap();
        assert!(super::write_new(path, b"replacement").is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"received attachment");
        std::fs::remove_file(path).unwrap();
    }
}
