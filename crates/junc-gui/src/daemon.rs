// SPDX-License-Identifier: GPL-3.0-or-later

//! The IPC client (architecture §7.1): the GUI never edits files; it reads
//! juncd's `Describe` snapshot and re-reads it whenever juncd says the
//! graph, the config, the Junction list or the captured apps changed, or
//! when juncd starts or stops. External changes (CLI, agents, hand edits)
//! therefore appear live.

use futures_util::StreamExt;
use futures_util::stream::{BoxStream, select_all};
use junc_ipc::DaemonProxy;
use junc_ipc::view::Describe;

async fn describe(p: &DaemonProxy<'_>) -> Option<Describe> {
    let text = p.describe().await.ok()?;
    serde_json::from_str(&text).ok()
}

/// Starts a thread that calls `update` with juncd's snapshot at once and
/// after every change, and with `None` while juncd is away. `address` is a
/// bus address (tests); `None` means the session bus.
pub fn follow(
    address: Option<String>,
    update: impl Fn(Option<Describe>) + Send + 'static,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("junc-daemon".into())
        .spawn(move || {
            let run = async {
                let conn = match &address {
                    Some(a) => {
                        zbus::connection::Builder::address(a.as_str())
                            .build()
                            .await?
                    }
                    None => zbus::Connection::session().await?,
                };
                // Property changes are reported only for a caching proxy.
                let p = DaemonProxy::new(&conn).await?;
                update(describe(&p).await);
                let graph = p.receive_graph_changed().await?.map(|_| true);
                let notice = p.receive_notice().await?.map(|_| true);
                let capture = p.receive_capture_apps_changed().await.map(|_| true);
                let revision = p.receive_revision_changed().await.map(|_| true);
                let list = p.receive_junctions_changed().await.map(|_| true);
                let owner = p
                    .inner()
                    .receive_owner_changed()
                    .await?
                    .map(|o| o.is_some());
                let streams: Vec<BoxStream<'_, bool>> = vec![
                    graph.boxed(),
                    notice.boxed(),
                    capture.boxed(),
                    revision.boxed(),
                    list.boxed(),
                    owner.boxed(),
                ];
                let mut all = select_all(streams);
                while let Some(running) = all.next().await {
                    update(if running { describe(&p).await } else { None });
                }
                zbus::Result::Ok(())
            };
            if zbus::block_on(run).is_err() {
                // No session bus: the window shows juncd as unreachable.
                update(None);
            }
        })?;
    Ok(())
}

/// Starts a thread that calls `notice` with the kind and JSON body of
/// every `Notice` juncd sends (`link_overridden`, `default_restored`, …;
/// boards 38, 39). `address` as for [`follow`].
pub fn notices(
    address: Option<String>,
    notice: impl Fn(String, String) + Send + 'static,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("junc-notices".into())
        .spawn(move || {
            let run = async {
                let conn = match &address {
                    Some(a) => {
                        zbus::connection::Builder::address(a.as_str())
                            .build()
                            .await?
                    }
                    None => zbus::Connection::session().await?,
                };
                let p = DaemonProxy::new(&conn).await?;
                let mut stream = p.receive_notice().await?;
                while let Some(n) = stream.next().await {
                    if let Ok(args) = n.args() {
                        notice(args.kind().to_string(), args.body().to_string());
                    }
                }
                zbus::Result::Ok(())
            };
            let _ = zbus::block_on(run);
        })?;
    Ok(())
}
