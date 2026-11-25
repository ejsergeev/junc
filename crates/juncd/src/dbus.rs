// SPDX-License-Identifier: GPL-3.0-or-later

//! The D-Bus API v1 (docs/dbus.md) on the session bus.
//!
//! zbus serves calls on its own helper thread. Calls that read or change the
//! config are forwarded to the main loop as a [`Req`] with a [`Responder`],
//! so every change happens on one thread in order with graph events.
//! Properties are read from a [`Shared`] snapshot the main loop publishes;
//! publishing emits `PropertiesChanged` and adds or removes Junction
//! objects.

use junc_ipc::view::JunctionView;
use junc_ipc::{API_VERSION, BUS_NAME, Error, ErrorBody, ROOT_PATH};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::object_server::SignalEmitter;
use zbus::wire::OwnedObjectPath;
use zbus::{Connection, interface};

/// A call the main loop answers.
#[derive(Debug)]
pub enum Req {
    Describe,
    ScanImportable,
    GetConfig,
    Validate(String),
    Plan(String),
    Apply {
        text: String,
        expected: u64,
    },
    Create {
        name: String,
    },
    ResolveLink {
        junction: String,
        reference: String,
        action: String,
    },
    Edit {
        id: String,
        edit: Edit,
    },
}

/// A change to one Junction.
#[derive(Debug)]
pub enum Edit {
    Rename(String),
    Enabled(bool),
    Volume(u32),
    Default(String),
    Delete(bool),
}

/// What the main loop answers.
#[derive(Debug)]
pub enum Resp {
    Text(String),
    Config(String, u64),
    Revision(u64),
    Applied(u64, String),
    Created(String, u64),
}

/// The main loop's answer.
pub type Reply = Result<Resp, ErrorBody>;

#[derive(Default)]
struct Slot {
    value: Option<Reply>,
    waker: Option<Waker>,
    closed: bool,
}

/// Sends one reply back to the waiting D-Bus call. Dropping it unanswered
/// fails the call instead of leaving it hanging.
pub struct Responder(Option<Arc<Mutex<Slot>>>);

impl Responder {
    /// Answers the call.
    pub fn send(mut self, reply: Reply) {
        self.finish(Some(reply));
    }

    fn finish(&mut self, reply: Option<Reply>) {
        if let Some(slot) = self.0.take() {
            let mut s = slot.lock().unwrap_or_else(PoisonError::into_inner);
            s.value = reply;
            s.closed = true;
            if let Some(w) = s.waker.take() {
                w.wake();
            }
        }
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.finish(None);
    }
}

struct Pending(Arc<Mutex<Slot>>);

impl Future for Pending {
    type Output = Option<Reply>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut s = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if s.closed {
            return Poll::Ready(s.value.take());
        }
        s.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Hands a request to the main loop.
pub type Dispatch = Arc<dyn Fn(Req, Responder) + Send + Sync>;

async fn call(dispatch: &Dispatch, req: Req) -> Result<Resp, Error> {
    let slot = Arc::new(Mutex::new(Slot::default()));
    dispatch(req, Responder(Some(slot.clone())));
    match Pending(slot).await {
        Some(Ok(r)) => Ok(r),
        Some(Err(body)) => Err(body.into()),
        None => Err(unexpected()),
    }
}

fn unexpected() -> Error {
    Error::from_body(&ErrorBody::new(
        "E_INTERNAL",
        "the daemon dropped the call",
        Some("See `journalctl --user -u juncd`."),
    ))
}

fn revision(r: Resp) -> Result<u64, Error> {
    match r {
        Resp::Revision(rev) => Ok(rev),
        _ => Err(unexpected()),
    }
}

fn text(r: Resp) -> Result<String, Error> {
    match r {
        Resp::Text(t) => Ok(t),
        _ => Err(unexpected()),
    }
}

fn object_path(p: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(p.to_owned()).unwrap_or_else(|_| {
        OwnedObjectPath::try_from(ROOT_PATH.to_owned()).expect("ROOT_PATH is valid")
    })
}

/// What properties are read from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shared {
    pub state: String,
    pub revision: u64,
    /// JSON array of diagnostics, `""` when the file is valid.
    pub config_error: String,
    pub junctions: Vec<JunctionView>,
    pub capture_apps: Vec<String>,
}

type SharedRef = Arc<Mutex<Shared>>;

fn read<T>(shared: &SharedRef, f: impl FnOnce(&Shared) -> T) -> T {
    f(&shared.lock().unwrap_or_else(PoisonError::into_inner))
}

struct DaemonIface {
    dispatch: Dispatch,
    shared: SharedRef,
}

#[interface(name = "io.github.ejsergeev.Junc1.Daemon")]
impl DaemonIface {
    async fn describe(&self) -> Result<String, Error> {
        text(call(&self.dispatch, Req::Describe).await?)
    }

    /// Existing null sinks and loopbacks that can become Junctions or
    /// Cables (F13), as JSON.
    async fn scan_importable(&self) -> Result<String, Error> {
        text(call(&self.dispatch, Req::ScanImportable).await?)
    }

    async fn get_config(&self) -> Result<(String, u64), Error> {
        match call(&self.dispatch, Req::GetConfig).await? {
            Resp::Config(t, rev) => Ok((t, rev)),
            _ => Err(unexpected()),
        }
    }

    async fn validate(&self, toml: String) -> Result<String, Error> {
        text(call(&self.dispatch, Req::Validate(toml)).await?)
    }

    async fn plan(&self, toml: String) -> Result<String, Error> {
        text(call(&self.dispatch, Req::Plan(toml)).await?)
    }

    async fn apply(&self, toml: String, expected_revision: u64) -> Result<(u64, String), Error> {
        let req = Req::Apply {
            text: toml,
            expected: expected_revision,
        };
        match call(&self.dispatch, req).await? {
            Resp::Applied(rev, plan) => Ok((rev, plan)),
            _ => Err(unexpected()),
        }
    }

    async fn create_junction(&self, name: String) -> Result<(OwnedObjectPath, u64), Error> {
        match call(&self.dispatch, Req::Create { name }).await? {
            Resp::Created(path, rev) => Ok((object_path(&path), rev)),
            _ => Err(unexpected()),
        }
    }

    async fn resolve_link(
        &self,
        junction: String,
        reference: String,
        action: String,
    ) -> Result<u64, Error> {
        let req = Req::ResolveLink {
            junction,
            reference,
            action,
        };
        revision(call(&self.dispatch, req).await?)
    }

    #[zbus(signal)]
    async fn graph_changed(emitter: &SignalEmitter<'_>, generation: u64) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn notice(emitter: &SignalEmitter<'_>, kind: &str, body: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn api_version(&self) -> (u32, u32) {
        API_VERSION
    }

    #[zbus(property)]
    fn daemon_version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_owned()
    }

    #[zbus(property)]
    fn state(&self) -> String {
        read(&self.shared, |s| s.state.clone())
    }

    #[zbus(property)]
    fn revision(&self) -> u64 {
        read(&self.shared, |s| s.revision)
    }

    #[zbus(property)]
    fn config_error(&self) -> String {
        read(&self.shared, |s| s.config_error.clone())
    }

    #[zbus(property)]
    fn junctions(&self) -> Vec<OwnedObjectPath> {
        read(&self.shared, |s| {
            s.junctions.iter().map(|j| object_path(&j.path)).collect()
        })
    }

    #[zbus(property)]
    fn capture_apps(&self) -> Vec<String> {
        read(&self.shared, |s| s.capture_apps.clone())
    }
}

struct JunctionIface {
    id: String,
    dispatch: Dispatch,
    shared: SharedRef,
}

impl JunctionIface {
    async fn edit(&self, edit: Edit) -> Result<u64, Error> {
        let req = Req::Edit {
            id: self.id.clone(),
            edit,
        };
        revision(call(&self.dispatch, req).await?)
    }

    fn get<T: Default>(&self, f: impl FnOnce(&JunctionView) -> T) -> T {
        read(&self.shared, |s| {
            s.junctions
                .iter()
                .find(|j| j.id == self.id)
                .map(f)
                .unwrap_or_default()
        })
    }
}

#[interface(name = "io.github.ejsergeev.Junc1.Junction")]
impl JunctionIface {
    async fn rename(&self, name: String) -> Result<u64, Error> {
        self.edit(Edit::Rename(name)).await
    }

    async fn set_enabled(&self, enabled: bool) -> Result<u64, Error> {
        self.edit(Edit::Enabled(enabled)).await
    }

    async fn set_volume(&self, percent: u32) -> Result<u64, Error> {
        self.edit(Edit::Volume(percent)).await
    }

    async fn set_default(&self, role: String) -> Result<u64, Error> {
        self.edit(Edit::Default(role)).await
    }

    async fn delete(&self, confirm: bool) -> Result<u64, Error> {
        self.edit(Edit::Delete(confirm)).await
    }

    #[zbus(property)]
    fn view(&self) -> String {
        self.get(|j| serde_json::to_string(j).unwrap_or_default())
    }

    #[zbus(property)]
    fn id(&self) -> String {
        self.id.clone()
    }

    #[zbus(property)]
    fn name(&self) -> String {
        self.get(|j| j.name.clone())
    }

    #[zbus(property)]
    fn enabled(&self) -> bool {
        self.get(|j| j.enabled)
    }
}

/// The daemon's end of the bus, used from the main thread.
pub struct Server {
    conn: Connection,
    dispatch: Dispatch,
    shared: SharedRef,
    published: Shared,
    generation: u64,
}

impl Server {
    /// Connects to the session bus, serves the daemon and Junction objects
    /// with `initial` as their state, and only then takes the well-known
    /// name, so the first call (often the one that activated the daemon)
    /// never sees an empty daemon. Fails if another juncd owns the name.
    pub fn start(dispatch: Dispatch, initial: Shared) -> zbus::Result<Self> {
        let shared = SharedRef::default();
        let iface = DaemonIface {
            dispatch: dispatch.clone(),
            shared: shared.clone(),
        };
        let conn = zbus::block_on(
            zbus::connection::Builder::session()
                .serve_at(ROOT_PATH, iface)
                .build(),
        )?;
        let mut server = Self {
            conn,
            dispatch,
            shared,
            published: Shared::default(),
            generation: 0,
        };
        server.publish(initial)?;
        zbus::block_on(async {
            // Explicit DO_NOT_QUEUE: the builder's default flags queue a
            // second daemon behind the first instead of failing.
            let reply = server
                .conn
                .request_name_with_flags(BUS_NAME, RequestNameFlags::DO_NOT_QUEUE)
                .await?;
            match reply {
                RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => Ok(()),
                _ => Err(zbus::Error::NameTaken),
            }
        })?;
        Ok(server)
    }

    fn emitter(&self, path: &str) -> zbus::Result<SignalEmitter<'static>> {
        SignalEmitter::new(&self.conn, path.to_owned())
    }

    /// Makes `next` what clients see, with change signals for what differs.
    pub fn publish(&mut self, next: Shared) -> zbus::Result<()> {
        if next == self.published {
            return Ok(());
        }
        *self.shared.lock().unwrap_or_else(PoisonError::into_inner) = next.clone();
        let prev = std::mem::replace(&mut self.published, next);
        let next = &self.published;
        zbus::block_on(async {
            let server = self.conn.object_server();
            for j in &prev.junctions {
                if !next.junctions.iter().any(|n| n.id == j.id) {
                    server.remove::<JunctionIface, _>(j.path.as_str()).await?;
                }
            }
            for j in &next.junctions {
                match prev.junctions.iter().find(|p| p.id == j.id) {
                    None => {
                        let iface = JunctionIface {
                            id: j.id.clone(),
                            dispatch: self.dispatch.clone(),
                            shared: self.shared.clone(),
                        };
                        server.at(j.path.as_str(), iface).await?;
                    }
                    Some(p) if p != j => {
                        let r = server
                            .interface::<_, JunctionIface>(j.path.as_str())
                            .await?;
                        let e = r.signal_emitter();
                        let iface = r.get().await;
                        iface.view_changed(e).await?;
                        if p.name != j.name {
                            iface.name_changed(e).await?;
                        }
                        if p.enabled != j.enabled {
                            iface.enabled_changed(e).await?;
                        }
                    }
                    Some(_) => {}
                }
            }
            let r = server.interface::<_, DaemonIface>(ROOT_PATH).await?;
            let e = r.signal_emitter();
            let iface = r.get().await;
            if prev.state != next.state {
                iface.state_changed(e).await?;
            }
            if prev.revision != next.revision {
                iface.revision_changed(e).await?;
            }
            if prev.config_error != next.config_error {
                iface.config_error_changed(e).await?;
            }
            let ids = |s: &Shared| s.junctions.iter().map(|j| j.id.clone()).collect::<Vec<_>>();
            if ids(&prev) != ids(next) {
                iface.junctions_changed(e).await?;
            }
            if prev.capture_apps != next.capture_apps {
                iface.capture_apps_changed(e).await?;
            }
            Ok(())
        })
    }

    /// Tells clients the routing state changed (statuses, matched nodes).
    pub fn graph_changed(&mut self) -> zbus::Result<()> {
        self.generation += 1;
        let e = self.emitter(ROOT_PATH)?;
        zbus::block_on(DaemonIface::graph_changed(&e, self.generation))
    }

    /// Sends a notice (`link_overridden`, `config_error`, …) with a JSON body.
    pub fn notice(&self, kind: &str, body: &serde_json::Value) -> zbus::Result<()> {
        let e = self.emitter(ROOT_PATH)?;
        zbus::block_on(DaemonIface::notice(&e, kind, &body.to_string()))
    }
}
