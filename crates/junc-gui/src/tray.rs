// SPDX-License-Identifier: GPL-3.0-or-later

//! The tray (architecture §7.8, design board 19): a StatusNotifierItem
//! whose menu is a DBusMenu, written on zbus (the owner chose this over
//! `ksni`, whose licence and second zbus the project does not take). The
//! desktop's shell draws the menu; JUNC serves its tree: "Capturing: …",
//! one check item per Junction or Cable, Open JUNC and Quit. The window
//! builds the menu (it has the translations) and hands it over; clicks
//! come back as [`Event`]s.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::object_server::SignalEmitter;
use zbus::{OwnedVariant, Structure, Variant};
use zbus::{fdo, interface};

/// One row of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// Text that cannot be clicked ("Capturing: …", a heading).
    Label(String),
    /// A Junction or Cable: its id, its name, on or off.
    Junction {
        id: String,
        label: String,
        on: bool,
    },
    /// Open JUNC, the default action.
    Open(String),
    /// Quit the GUI (juncd keeps running).
    Quit(String),
    Separator,
}

/// What a click asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Turn Junction `0` on or off.
    Toggle(String, bool),
    Open,
    Quit,
}

type Shared = Arc<Mutex<(u32, Vec<Entry>)>>;

/// The menu's id of entry `i` (0 is the root).
fn menu_id(i: usize) -> i32 {
    i32::try_from(i + 1).unwrap_or(i32::MAX)
}

/// The DBusMenu properties of one entry.
fn props(e: &Entry) -> HashMap<String, OwnedVariant> {
    let mut p: HashMap<String, OwnedVariant> = HashMap::new();
    let mut put = |k: &str, v: Variant<'_>| {
        if let Ok(v) = OwnedVariant::try_from(v) {
            p.insert(k.to_owned(), v);
        }
    };
    match e {
        Entry::Separator => put("type", Variant::from("separator")),
        Entry::Label(t) => {
            put("label", Variant::from(t.as_str()));
            put("enabled", Variant::from(false));
        }
        Entry::Junction { label, on, .. } => {
            put("label", Variant::from(label.as_str()));
            put("toggle-type", Variant::from("checkmark"));
            put("toggle-state", Variant::from(i32::from(*on)));
        }
        Entry::Open(t) | Entry::Quit(t) => put("label", Variant::from(t.as_str())),
    }
    p
}

/// One node of the layout: (id, properties, children as variants of
/// nodes), DBusMenu's `(ia{sv}av)`.
pub type Node = (i32, HashMap<String, OwnedVariant>, Vec<OwnedVariant>);

/// The whole tree under the root, as `GetLayout` returns it.
pub fn layout(entries: &[Entry]) -> Node {
    let children = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let child: Node = (menu_id(i), props(e), Vec::new());
            let s: Structure<'static> = Structure::from(child);
            OwnedVariant::try_from(Variant::from(s)).ok()
        })
        .collect();
    let mut root = HashMap::new();
    if let Ok(v) = OwnedVariant::try_from(Variant::from("submenu")) {
        root.insert("children-display".to_owned(), v);
    }
    (0, root, children)
}

struct Menu {
    shared: Shared,
    events: Arc<dyn Fn(Event) + Send + Sync>,
}

#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
    fn get_layout(&self, _parent: i32, _depth: i32, _names: Vec<String>) -> (u32, Node) {
        let s = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (s.0, layout(&s.1))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedVariant>)> {
        let s = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        s.1.iter()
            .enumerate()
            .map(|(i, e)| (menu_id(i), e))
            .filter(|(id, _)| ids.is_empty() || ids.contains(id))
            .map(|(id, e)| (id, props(e)))
            .collect()
    }

    fn get_property(&self, id: i32, name: String) -> fdo::Result<OwnedVariant> {
        let s = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        usize::try_from(id - 1)
            .ok()
            .and_then(|i| s.1.get(i))
            .and_then(|e| props(e).remove(&name))
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("no {name} on {id}")))
    }

    fn event(&self, id: i32, event_id: String, _data: Variant<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        let entry = {
            let s = self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            usize::try_from(id - 1)
                .ok()
                .and_then(|i| s.1.get(i).cloned())
        };
        match entry {
            Some(Entry::Junction { id, on, .. }) => (self.events)(Event::Toggle(id, !on)),
            Some(Entry::Open(_)) => (self.events)(Event::Open),
            Some(Entry::Quit(_)) => (self.events)(Event::Quit),
            _ => {}
        }
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedVariant, u32)>) -> Vec<i32> {
        for (id, event_id, data, timestamp) in events {
            self.event(id, event_id, Variant::from(data), timestamp);
        }
        Vec::new()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }
}

struct Item {
    events: Arc<dyn Fn(Event) + Send + Sync>,
}

#[interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    /// A click on the icon opens JUNC.
    fn activate(&self, _x: i32, _y: i32) {
        (self.events)(Event::Open);
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {}

    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, _delta: i32, _orientation: String) {}

    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "junc"
    }

    #[zbus(property)]
    fn title(&self) -> &str {
        "JUNC"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        "junc-symbolic"
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn menu(&self) -> zbus::OwnedObjectPath {
        zbus::OwnedObjectPath::try_from(MENU).unwrap_or_else(|e| unreachable!("a fixed path: {e}"))
    }
}

const ITEM: &str = "/StatusNotifierItem";
const MENU: &str = "/MenuBar";

/// The served tray; dropping it removes it.
pub struct Tray {
    conn: zbus::Connection,
    shared: Shared,
}

impl std::fmt::Debug for Tray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tray").finish_non_exhaustive()
    }
}

impl Tray {
    /// Serves the tray on `address` (tests) or the session bus and asks
    /// the shell's StatusNotifierWatcher to show it. Fails without a bus;
    /// without a watcher it is served but shown nowhere (`registered`
    /// says which).
    pub fn start(
        address: Option<&str>,
        events: impl Fn(Event) + Send + Sync + 'static,
    ) -> zbus::Result<(Self, bool)> {
        let events: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(events);
        let shared: Shared = Arc::new(Mutex::new((1, Vec::new())));
        let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
        let (conn, registered) = zbus::block_on(async {
            let builder = match address {
                Some(a) => zbus::connection::Builder::address(a),
                None => zbus::connection::Builder::session(),
            };
            let conn = builder
                .name(name.as_str())
                .serve_at(
                    ITEM,
                    Item {
                        events: events.clone(),
                    },
                )
                .serve_at(
                    MENU,
                    Menu {
                        shared: shared.clone(),
                        events,
                    },
                )
                .build()
                .await?;
            let registered = conn
                .call_method(
                    Some("org.kde.StatusNotifierWatcher"),
                    "/StatusNotifierWatcher",
                    Some("org.kde.StatusNotifierWatcher"),
                    "RegisterStatusNotifierItem",
                    &(name.as_str()),
                )
                .await
                .is_ok();
            zbus::Result::Ok((conn, registered))
        })?;
        Ok((Self { conn, shared }, registered))
    }

    /// Shows `entries` as the menu (a new layout revision).
    pub fn set_menu(&self, entries: Vec<Entry>) {
        let revision = {
            let mut s = self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if s.1 == entries {
                return;
            }
            s.0 += 1;
            s.1 = entries;
            s.0
        };
        let server = self.conn.object_server();
        let _ = zbus::block_on(async {
            let iface = server.interface::<_, Menu>(MENU).await?;
            Menu::layout_updated(iface.signal_emitter(), revision, 0).await
        });
    }

    /// The bus name the tray is served under.
    pub fn name(&self) -> String {
        format!("org.kde.StatusNotifierItem-{}-1", std::process::id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_has_one_node_per_entry_with_its_properties() {
        let entries = vec![
            Entry::Label("Capturing: Spotify".into()),
            Entry::Separator,
            Entry::Junction {
                id: "zoom".into(),
                label: "Zoom Mic".into(),
                on: true,
            },
            Entry::Open("Open JUNC".into()),
        ];
        let (id, _, children) = layout(&entries);
        assert_eq!(id, 0);
        assert_eq!(children.len(), 4);
        let p = props(&entries[2]);
        assert_eq!(
            p.get("toggle-state").map(|v| i32::try_from(v).ok()),
            Some(Some(1))
        );
        assert_eq!(
            props(&entries[0])
                .get("enabled")
                .map(|v| bool::try_from(v).ok()),
            Some(Some(false))
        );
    }
}
