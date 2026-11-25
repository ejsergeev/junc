// SPDX-License-Identifier: GPL-3.0-or-later

//! The D-Bus API v1 end to end: juncd on a private bus (its own
//! `dbus-daemon`), no PipeWire (an empty runtime dir), driven through the
//! `junc-ipc` proxies the CLI and GUI use. Covers every call and error kind,
//! the single-instance rule and the call latency budget (< 50 ms p95).

use futures_util::StreamExt;
use junc_ipc::view::{Describe, Diag, State};
use junc_ipc::{DaemonProxy, ErrorBody, JunctionProxy, error_body};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use zbus::Connection;
use zbus::proxy::CacheProperties;

const CONFIG: &str = r#"# Hand-written; keep this note.
schema = 1

[[junction]]
id = "zoom"
name = "Zoom Mic" # the meeting one
"#;

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Env {
    dir: PathBuf,
    address: String,
    _bus: Kill,
    _daemon: Kill,
}

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("juncd-dbus-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("run")).expect("tmp dir");
    dir
}

/// Kills a process we did not spawn.
struct KillPid(u32);
impl Drop for KillPid {
    fn drop(&mut self) {
        let _ = Command::new("kill").arg(self.0.to_string()).status();
    }
}

/// A private session bus, or `None` when `dbus-daemon` is not installed.
fn private_bus() -> Option<(Kill, String)> {
    let mut child = Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--nopidfile", "--print-address=1"])
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let mut line = String::new();
    BufReader::new(child.stdout.take()?)
        .read_line(&mut line)
        .ok()?;
    Some((Kill(child), line.trim().to_owned()))
}

fn juncd(dir: &Path, address: &str, log: &str) -> Command {
    let mut c = juncd_on_pipewire(dir, address, log);
    // No PipeWire: the daemon waits for a socket that never comes.
    c.env("PIPEWIRE_RUNTIME_DIR", dir.join("run"))
        .env_remove("PIPEWIRE_REMOTE");
    c
}

fn juncd_on_pipewire(dir: &Path, address: &str, log: &str) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_juncd"));
    c.arg("--config")
        .arg(dir.join("junctions.toml"))
        .arg("--state-dir")
        .arg(dir)
        .env("DBUS_SESSION_BUS_ADDRESS", address)
        .stderr(std::fs::File::create(dir.join(log)).expect("log"));
    c
}

fn start(tag: &str) -> Option<Env> {
    start_with(tag, juncd)
}

fn start_with(tag: &str, run: fn(&Path, &str, &str) -> Command) -> Option<Env> {
    let Some((bus, address)) = private_bus() else {
        assert!(
            std::env::var_os("CI").is_none(),
            "dbus-daemon is missing in CI"
        );
        eprintln!("skipped: no dbus-daemon");
        return None;
    };
    let dir = tmp(tag);
    std::fs::write(dir.join("junctions.toml"), CONFIG).expect("config");
    let daemon = Kill(run(&dir, &address, "juncd.log").spawn().expect("juncd"));
    Some(Env {
        dir,
        address,
        _bus: bus,
        _daemon: daemon,
    })
}

async fn connect(env: &Env) -> (Connection, DaemonProxy<'static>) {
    let conn = zbus::connection::Builder::address(env.address.as_str())
        .build()
        .await
        .expect("connect");
    let proxy = DaemonProxy::builder(&conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .expect("proxy");
    let until = Instant::now() + Duration::from_secs(10);
    while proxy.revision().await.is_err() {
        assert!(Instant::now() < until, "juncd did not appear on the bus");
        std::thread::sleep(Duration::from_millis(20));
    }
    (conn, proxy)
}

async fn junction<'a>(conn: &Connection, path: &str) -> JunctionProxy<'a> {
    JunctionProxy::builder(conn)
        .path(path.to_owned())
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .expect("junction proxy")
}

/// Code of the error a call failed with.
fn code<T: std::fmt::Debug>(r: zbus::Result<T>) -> ErrorBody {
    error_body(&r.expect_err("the call should fail"))
}

fn dbus_name<T: std::fmt::Debug>(r: &zbus::Result<T>) -> String {
    match r {
        Err(zbus::Error::MethodError(name, _, _)) => name.to_string(),
        other => panic!("expected a D-Bus error, got {other:?}"),
    }
}

#[test]
fn every_call_round_trips() {
    let Some(env) = start("calls") else { return };
    zbus::block_on(async {
        let (conn, d) = connect(&env).await;
        assert_eq!(d.api_version().await.expect("D-Bus call"), (1, 1));
        assert_eq!(d.state().await.expect("D-Bus call"), "starting");
        assert_eq!(d.revision().await.expect("D-Bus call"), 1);
        assert_eq!(d.config_error().await.expect("D-Bus call"), "");
        assert!(d.capture_apps().await.expect("D-Bus call").is_empty());

        // Describe: config with statuses (unknown without PipeWire).
        let desc: Describe =
            serde_json::from_str(&d.describe().await.expect("D-Bus call")).expect("JSON");
        assert_eq!(desc.junctions.len(), 1);
        assert_eq!(desc.junctions[0].node, "junc.zoom");
        assert_eq!(
            desc.junctions[0].path,
            "/io/github/ejsergeev/Junc1/junction/zoom"
        );
        assert_eq!(desc.revision, 1);

        // Read, validate, plan.
        let (text, rev) = d.get_config().await.expect("D-Bus call");
        assert_eq!((text.as_str(), rev), (CONFIG, 1));
        let diags: Vec<Diag> = serde_json::from_str(
            &d.validate("schema = 1\nvolme = 3\n")
                .await
                .expect("D-Bus call"),
        )
        .expect("JSON");
        assert_eq!(diags[0].code, "E_CONFIG_UNKNOWN_KEY");
        assert_eq!(diags[0].location.as_ref().expect("location").line, 2);
        assert_eq!(d.validate(CONFIG).await.expect("D-Bus call"), "[]");
        let r = d.plan(CONFIG).await;
        assert_eq!(
            dbus_name(&r),
            "io.github.ejsergeev.Junc1.Error.PipeWireUnavailable"
        );
        assert_eq!(code(r).code, "E_PIPEWIRE_UNAVAILABLE");

        // Create: the id comes from the name and the object appears.
        let (path, rev) = d.create_junction("Podcast Mix").await.expect("D-Bus call");
        assert_eq!(
            path.as_str(),
            "/io/github/ejsergeev/Junc1/junction/podcast_2dmix"
        );
        assert_eq!(rev, 2);
        let paths = d.junctions().await.expect("D-Bus call");
        assert_eq!(paths.len(), 2);
        let j = junction(&conn, path.as_str()).await;
        assert_eq!(j.id().await.expect("D-Bus call"), "podcast-mix");
        assert_eq!(j.name().await.expect("D-Bus call"), "Podcast Mix");

        // Edits go to the file and keep the user's comments.
        assert_eq!(j.set_volume(50).await.expect("D-Bus call"), 3);
        assert_eq!(j.rename("Podcast").await.expect("D-Bus call"), 4);
        assert_eq!(j.set_enabled(false).await.expect("D-Bus call"), 5);
        assert!(!j.enabled().await.expect("D-Bus call"));
        assert_eq!(j.set_default("input").await.expect("D-Bus call"), 6);
        let file = std::fs::read_to_string(env.dir.join("junctions.toml")).expect("config file");
        for want in [
            "# Hand-written; keep this note.",
            "# the meeting one",
            "name = \"Podcast\"",
            "volume = 50",
            "enabled = false",
            "default = \"input\"",
        ] {
            assert!(file.contains(want), "missing {want:?} in:\n{file}");
        }
        let view: serde_json::Value =
            serde_json::from_str(&j.view().await.expect("D-Bus call")).expect("JSON");
        assert_eq!(view["volume"], 50);

        // Errors: each with a code and a hint.
        let r = j.set_volume(900).await;
        assert_eq!(
            dbus_name(&r),
            "io.github.ejsergeev.Junc1.Error.InvalidConfig"
        );
        let e = code(r);
        assert_eq!(e.code, "E_CONFIG_RANGE");
        assert!(e.hint.is_some());
        assert_eq!(
            code(j.set_default("both").await).code,
            "E_CONFIG_INVALID_VALUE"
        );
        let r = j.delete(false).await;
        assert_eq!(
            dbus_name(&r),
            "io.github.ejsergeev.Junc1.Error.NeedsConfirmation"
        );
        let r = d.resolve_link("zoom", "in/mic/1->1", "leave").await;
        assert_eq!(dbus_name(&r), "io.github.ejsergeev.Junc1.Error.NotFound");
        assert_eq!(
            code(d.resolve_link("zoom", "x", "maybe").await).code,
            "E_CONFIG_INVALID_VALUE"
        );
        assert_eq!(
            d.revision().await.expect("D-Bus call"),
            6,
            "failed calls change nothing"
        );

        // Delete removes the object.
        assert_eq!(j.delete(true).await.expect("D-Bus call"), 7);
        assert_eq!(d.junctions().await.expect("D-Bus call").len(), 1);
        assert!(j.name().await.is_err());

        // Apply: stale revisions conflict; the text is written as given.
        let r = d.apply(CONFIG, 1).await;
        assert_eq!(dbus_name(&r), "io.github.ejsergeev.Junc1.Error.Conflict");
        assert_eq!(code(r).code, "E_CONFIG_CONFLICT");
        let mine = CONFIG.replace("Zoom Mic", "Meetings");
        let (rev, plan) = d.apply(&mine, 7).await.expect("D-Bus call");
        assert_eq!(rev, 8);
        let plan: serde_json::Value = serde_json::from_str(&plan).expect("JSON");
        assert!(
            plan["actions"].as_array().expect("actions").is_empty(),
            "no PipeWire, no actions"
        );
        assert_eq!(
            std::fs::read_to_string(env.dir.join("junctions.toml")).expect("config file"),
            mine
        );
        assert_eq!(
            code(d.apply("schema = 1\nfoo = 1\n", 0).await).code,
            "E_CONFIG_UNKNOWN_KEY"
        );

        // Hand edits show up in the properties; bad ones in ConfigError.
        std::fs::write(env.dir.join("junctions.toml"), CONFIG).expect("write config");
        wait(|| async { d.revision().await.expect("D-Bus call") == 9 }).await;
        std::fs::write(env.dir.join("junctions.toml"), "schema = 1\nvolme = 3\n")
            .expect("write config");
        wait(|| async { !d.config_error().await.expect("D-Bus call").is_empty() }).await;
        let err: Vec<Diag> =
            serde_json::from_str(&d.config_error().await.expect("D-Bus call")).expect("JSON");
        assert_eq!(err[0].code, "E_CONFIG_UNKNOWN_KEY");
        assert_eq!(
            d.junctions().await.expect("D-Bus call").len(),
            1,
            "last good config stays"
        );
    });
}

async fn wait<F, Fut>(mut f: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let until = Instant::now() + Duration::from_secs(5);
    while !f().await {
        assert!(Instant::now() < until, "timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_second_daemon_does_not_start() {
    let Some(env) = start("twice") else { return };
    let _client = zbus::block_on(connect(&env));
    let status = juncd(&env.dir, &env.address, "second.log")
        .status()
        .expect("second juncd");
    assert_eq!(status.code(), Some(4));
    let log = std::fs::read_to_string(env.dir.join("second.log")).expect("log");
    assert!(log.contains("io.github.ejsergeev.Junc1"), "{log}");
}

#[test]
fn calls_answer_within_50_ms() {
    let Some(env) = start("latency") else { return };
    zbus::block_on(async {
        let (conn, d) = connect(&env).await;
        let j = junction(&conn, "/io/github/ejsergeev/Junc1/junction/zoom").await;
        let mut times = Vec::new();
        for i in 0..100u32 {
            let t = Instant::now();
            d.describe().await.expect("D-Bus call");
            times.push(t.elapsed());
            let t = Instant::now();
            j.set_volume(50 + i % 2).await.expect("D-Bus call");
            times.push(t.elapsed());
        }
        times.sort();
        let p95 = times[times.len() * 95 / 100];
        eprintln!(
            "D-Bus call latency: p50 {:?}, p95 {p95:?}, max {:?}",
            times[times.len() / 2],
            times[times.len() - 1]
        );
        assert!(p95 < Duration::from_millis(50), "p95 {p95:?}");
    });
}

/// Hardware layer 2: with the isolated PipeWire of
/// `scripts/ci/live-pipewire.sh`, statuses reach clients and `GraphChanged`
/// fires when routing changes.
#[test]
fn live_statuses_and_graph_changed() {
    if std::env::var("JUNC_LIVE_PIPEWIRE").as_deref() != Ok("1") {
        eprintln!("skipped: run scripts/ci/live-pipewire.sh");
        return;
    }
    let Some(env) = start_with("live", juncd_on_pipewire) else {
        return;
    };
    zbus::block_on(async {
        let (_conn, d) = connect(&env).await;
        wait(|| async { d.state().await.expect("D-Bus call") == "connected" }).await;
        let mut changed = d.receive_graph_changed().await.expect("D-Bus call");
        let (_, rev) = d.get_config().await.expect("D-Bus call");
        let config = format!(
            "{CONFIG}\n[[junction.input]]\nid = \"mic\"\nmatch = {{ device = \"fake.hwin\" }}\n"
        );
        d.apply(&config, rev).await.expect("D-Bus call");
        let t = Instant::now();
        let signal = changed.next().await.expect("GraphChanged");
        assert!(signal.args().expect("args").generation > 0);
        eprintln!("GraphChanged {:?} after Apply", t.elapsed());
        wait(|| async {
            let desc: Describe =
                serde_json::from_str(&d.describe().await.expect("D-Bus call")).expect("JSON");
            let j = &desc.junctions[0];
            j.status == State::Active
                && j.inputs[0].status == State::Active
                && j.inputs[0].matched == ["fake.hwin"]
                && j.inputs[0].links == [[1, 1], [1, 2]]
        })
        .await;
        let desc: Describe =
            serde_json::from_str(&d.describe().await.expect("D-Bus call")).expect("JSON");
        assert!(
            desc.candidates
                .iter()
                .any(|c| c.node == "fake.hwin" && c.kind == "source")
        );
        assert!(
            desc.candidates
                .iter()
                .any(|c| c.node == "junc.zoom" && c.kind == "junction")
        );
        assert!(desc.defaults.output.is_some());
    });
}

/// Our interface's element from an introspection document.
fn interface_xml(doc: &str, name: &str) -> String {
    let start = doc
        .find(&format!("<interface name=\"{name}\">"))
        .expect("interface");
    let end = start + doc[start..].find("</interface>").expect("end") + "</interface>".len();
    doc[start..end].to_owned()
}

/// docs/dbus/io.github.ejsergeev.Junc1.xml is generated from the running
/// daemon; `JUNC_BLESS=1` rewrites it after an API change.
#[test]
fn introspection_xml_is_up_to_date() {
    let Some(env) = start("xml") else { return };
    let xml = zbus::block_on(async {
        let (conn, _d) = connect(&env).await;
        let mut parts = Vec::new();
        for (path, iface) in [
            (junc_ipc::ROOT_PATH, junc_ipc::DAEMON_IFACE),
            (
                "/io/github/ejsergeev/Junc1/junction/zoom",
                junc_ipc::JUNCTION_IFACE,
            ),
        ] {
            let p = zbus::fdo::IntrospectableProxy::builder(&conn)
                .destination(junc_ipc::BUS_NAME)
                .path(path)
                .build()
                .await
                .expect("introspection proxy");
            parts.push(interface_xml(
                &p.introspect().await.expect("D-Bus call"),
                iface,
            ));
        }
        parts
    });
    let doc = format!(
        "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\"\n \
         \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n\
         <!-- Generated from juncd by crates/juncd/tests/dbus.rs; do not edit.\n     \
         Regenerate: JUNC_BLESS=1 cargo test -p juncd --test dbus introspection -->\n\
         <node>\n  {}\n</node>\n",
        xml.join("\n  ")
    );
    let file =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/dbus/io.github.ejsergeev.Junc1.xml");
    if std::env::var_os("JUNC_BLESS").is_some() {
        std::fs::create_dir_all(file.parent().expect("docs dir")).expect("mkdir");
        std::fs::write(&file, &doc).expect("write XML");
    }
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    assert!(
        current == doc,
        "{} is out of date; run JUNC_BLESS=1 cargo test -p juncd --test dbus introspection",
        file.display()
    );
}

/// F9: under systemd (`Type=notify`) juncd says READY=1 once it serves
/// D-Bus, before PipeWire is there.
#[test]
fn juncd_notifies_systemd_when_ready() {
    let Some((bus, address)) = private_bus() else {
        eprintln!("skipped: no dbus-daemon");
        return;
    };
    let dir = tmp("notify");
    std::fs::write(dir.join("junctions.toml"), CONFIG).expect("config");
    let sock_path = dir.join("notify.sock");
    let sock = std::os::unix::net::UnixDatagram::bind(&sock_path).expect("bind");
    sock.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    let daemon = Kill(
        juncd(&dir, &address, "juncd.log")
            .env("NOTIFY_SOCKET", &sock_path)
            .spawn()
            .expect("juncd"),
    );
    let mut buf = [0u8; 512];
    let n = sock.recv(&mut buf).expect("a notification");
    let msg = String::from_utf8_lossy(&buf[..n]);
    assert!(msg.starts_with("READY=1\nSTATUS=1 Junction(s)"), "{msg}");
    // By then the API answers.
    let env = Env {
        dir,
        address,
        _bus: bus,
        _daemon: daemon,
    };
    zbus::block_on(async {
        let (_c, d) = connect(&env).await;
        assert_eq!(d.revision().await.expect("D-Bus call"), 1);
    });
}

/// The shipped activation file starts juncd on the first call (the CLI or
/// GUI never has to start it).
#[test]
fn the_activation_file_starts_juncd_on_demand() {
    let dir = tmp("activate");
    std::fs::write(dir.join("junctions.toml"), CONFIG).expect("config");
    let services = dir.join("services");
    std::fs::create_dir_all(&services).expect("dir");
    let shipped = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging/dbus/io.github.ejsergeev.Junc1.service"),
    )
    .expect("activation file");
    assert!(shipped.contains("Exec=/usr/bin/juncd\n"), "{shipped}");
    let exec = format!(
        "Exec={} --config {} --state-dir {}\n",
        env!("CARGO_BIN_EXE_juncd"),
        dir.join("junctions.toml").display(),
        dir.display()
    );
    std::fs::write(
        services.join("io.github.ejsergeev.Junc1.service"),
        shipped.replace("Exec=/usr/bin/juncd\n", &exec),
    )
    .expect("write");
    let conf = dir.join("bus.conf");
    std::fs::write(
        &conf,
        format!(
            r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir={}</listen>
  <servicedir>{}</servicedir>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
            dir.display(),
            services.display()
        ),
    )
    .expect("bus config");
    let Ok(mut child) = Command::new("dbus-daemon")
        .arg(format!("--config-file={}", conf.display()))
        .args(["--nofork", "--nopidfile", "--print-address=1"])
        // Activated services inherit this: no PipeWire for the daemon.
        .env("PIPEWIRE_RUNTIME_DIR", dir.join("run"))
        .stdout(Stdio::piped())
        // ...and these: keep them off the test's output.
        .stderr(Stdio::null())
        .spawn()
    else {
        eprintln!("skipped: no dbus-daemon");
        return;
    };
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut line)
        .expect("address");
    let _bus = Kill(child);
    let address = line.trim().to_owned();

    zbus::block_on(async {
        let conn = zbus::connection::Builder::address(address.as_str())
            .build()
            .await
            .expect("connect");
        // No daemon yet; the call itself starts it.
        let d = DaemonProxy::builder(&conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await
            .expect("proxy");
        let t = Instant::now();
        let revision = d.revision().await.expect("activated");
        eprintln!("activation answered after {:?}", t.elapsed());
        // The activated daemon is the bus's child, not ours: stop it
        // whatever the assertions below say.
        let dbus = zbus::fdo::DBusProxy::new(&conn).await.expect("dbus");
        let name = zbus::names::BusName::try_from(junc_ipc::BUS_NAME).expect("name");
        let pid = dbus
            .get_connection_unix_process_id(name)
            .await
            .expect("pid");
        let _stop = KillPid(pid);
        // The first call already sees the config, not an empty daemon.
        assert_eq!(revision, 1);
        assert_eq!(d.junctions().await.expect("junctions").len(), 1);
    });
    let _ = std::fs::remove_dir_all(&dir);
}
