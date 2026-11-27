// SPDX-License-Identifier: GPL-3.0-or-later

//! The tray (design board 19, architecture §7.8) as a shell sees it: the
//! DBusMenu tree on a private bus, its revision moving on with the menu,
//! and clicks coming back as events. No shell draws it here.

use junc_gui::tray::{Entry, Event, Node, Tray};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::time::Duration;
use zbus::Variant;

struct Kill(Child);
impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

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

/// The labels and toggle states of the root's children.
fn rows(layout: &Node) -> Vec<(String, Option<i32>, bool)> {
    layout
        .2
        .iter()
        .map(|c| {
            let (_, props, _): Node = c.try_clone().expect("clone").try_into().expect("a node");
            let text = |k: &str| {
                props
                    .get(k)
                    .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            };
            let toggle = props
                .get("toggle-state")
                .and_then(|v| i32::try_from(v).ok());
            (
                text("label").unwrap_or_default(),
                toggle,
                text("type").is_some_and(|t| t == "separator"),
            )
        })
        .collect()
}

#[test]
fn the_menu_is_a_dbusmenu_tree_and_clicks_come_back() {
    let Some((_bus, address)) = private_bus() else {
        eprintln!("skipped: no dbus-daemon");
        return;
    };
    let (tx, rx) = channel();
    let (tray, shown) = Tray::start(Some(&address), move |e| {
        let _ = tx.send(e);
    })
    .expect("tray");
    assert!(!shown, "no StatusNotifierWatcher on a private bus");
    tray.set_menu(vec![
        Entry::Label("Capturing: Spotify, Soundux".into()),
        Entry::Separator,
        Entry::Junction {
            id: "virtual-mic-for-zoom".into(),
            label: "Virtual Mic for Zoom".into(),
            on: true,
        },
        Entry::Junction {
            id: "firefox-to-obs".into(),
            label: "Firefox → OBS".into(),
            on: false,
        },
        Entry::Separator,
        Entry::Open("Open JUNC".into()),
        Entry::Quit("Quit".into()),
    ]);

    let conn = zbus::block_on(zbus::connection::Builder::address(address.as_str()).build())
        .expect("client");
    let dest = tray.name();
    let menu = |method: &str, body: &(i32, &str, Variant<'_>, u32)| {
        zbus::block_on(conn.call_method(
            Some(dest.as_str()),
            "/MenuBar",
            Some("com.canonical.dbusmenu"),
            method,
            body,
        ))
        .expect(method)
    };
    let reply = zbus::block_on(conn.call_method(
        Some(dest.as_str()),
        "/MenuBar",
        Some("com.canonical.dbusmenu"),
        "GetLayout",
        &(0_i32, -1_i32, Vec::<String>::new()),
    ))
    .expect("GetLayout");
    let (revision, layout): (u32, Node) = reply.body().deserialize().expect("layout");
    assert!(revision >= 2, "a menu was set");
    let got = rows(&layout);
    assert_eq!(got.len(), 7);
    assert_eq!(got[0].0, "Capturing: Spotify, Soundux");
    assert!(got[1].2, "a separator");
    assert_eq!(got[2], ("Virtual Mic for Zoom".into(), Some(1), false));
    assert_eq!(got[3], ("Firefox → OBS".into(), Some(0), false));
    assert_eq!(got[5].0, "Open JUNC");

    // A click on Firefox → OBS (id 4) turns it on; Open JUNC opens.
    menu("Event", &(4, "clicked", Variant::from(0_i32), 0));
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).expect("event"),
        Event::Toggle("firefox-to-obs".into(), true)
    );
    menu("Event", &(6, "clicked", Variant::from(0_i32), 0));
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).expect("event"),
        Event::Open
    );
    // Clicking the icon opens JUNC too.
    zbus::block_on(conn.call_method(
        Some(dest.as_str()),
        "/StatusNotifierItem",
        Some("org.kde.StatusNotifierItem"),
        "Activate",
        &(0_i32, 0_i32),
    ))
    .expect("Activate");
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).expect("event"),
        Event::Open
    );
}
