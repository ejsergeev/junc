// SPDX-License-Identifier: GPL-3.0-or-later

//! The window's controller (architecture §7.1): keeps juncd's latest
//! `Describe`, the selected Junction, its Main model and layout, the link
//! under the pointer, onboarding and the Add Input picker, and turns the
//! window's callbacks into [`Command`]s. Everything runs on the GUI thread.

use crate::actions::{Actions, Command};
use crate::checks;
use crate::edits::{Edit, Part};
use crate::layout::{self, Layout, Point};
use crate::main_view::{self, End, Main};
use crate::meter_service::{Levels, MeterService};
use crate::onboarding::{self, Step};
use crate::picker::{self, Entry, Pick};
use crate::state::{self, Snapshot};
use crate::ui::{self, AppWindow, View, Words};
use junc_ipc::view::Describe;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// Volume sliders send their value once it rests this long.
const SETTLE: Duration = Duration::from_millis(250);

/// The controller.
pub struct App {
    window: slint::Weak<AppWindow>,
    actions: Rc<dyn Actions>,
    meters: Option<Rc<MeterService>>,
    levels: Levels,
    describe: Option<Describe>,
    sidebar: Snapshot,
    selected: Option<String>,
    main: Option<Main>,
    layout: Option<Layout>,
    /// The canvas width, set by the window (also while the controller is
    /// busy, so it lives outside the RefCell).
    width: Rc<Cell<f32>>,
    hovered: Option<usize>,
    /// Volumes waiting for their slider to rest, by what they change.
    pending: Rc<RefCell<BTreeMap<String, Command>>>,
    timer: slint::Timer,
    /// Onboarding's step on screen, if any.
    onboarding: Option<Step>,
    /// Where onboarding is remembered as done; `None` never onboards
    /// (scenarios set the step themselves).
    state_file: Option<PathBuf>,
    /// The system check's last answer.
    checks: Vec<checks::Row>,
    /// The Junction ids before a New Junction, to select the new one.
    creating: Option<Vec<String>>,
    /// The picker's rows and those the search shows, when it is open.
    picker: Option<(Vec<Entry>, Vec<usize>)>,
    /// The open picker adds Listen outputs (board 17), not Inputs.
    picker_listen: bool,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("selected", &self.selected)
            .field("hovered", &self.hovered)
            .finish_non_exhaustive()
    }
}

/// Wires `window`'s callbacks to a new controller.
pub fn install(
    window: &AppWindow,
    actions: Rc<dyn Actions>,
    meters: Option<Rc<MeterService>>,
    levels: Levels,
) -> Rc<RefCell<App>> {
    let view = window.global::<View>();
    view.set_levels(ModelRc::from(levels.levels.clone()));
    view.set_peaks(ModelRc::from(levels.peaks.clone()));
    let width = Rc::new(Cell::new(0.0));
    let app = Rc::new(RefCell::new(App {
        window: window.as_weak(),
        actions,
        meters,
        levels,
        describe: None,
        sidebar: state::snapshot(None),
        selected: None,
        main: None,
        layout: None,
        width: width.clone(),
        hovered: None,
        pending: Rc::default(),
        timer: slint::Timer::default(),
        onboarding: None,
        state_file: None,
        checks: Vec::new(),
        creating: None,
        picker: None,
        picker_listen: false,
    }));
    // Runs `f` on the controller unless it is busy (a callback raised while
    // it updates the window, such as a relayout when the canvas appears).
    let w = Rc::downgrade(&app);
    let call = move |f: &dyn Fn(&mut App)| {
        if let Some(a) = w.upgrade()
            && let Ok(mut a) = a.try_borrow_mut()
        {
            f(&mut a);
        }
    };
    let call = Rc::new(call);

    {
        let c = call.clone();
        view.on_select(move |id| c(&|a| a.select(id.as_str())));
    }
    {
        let c = call.clone();
        view.on_junction_toggled(move |id, on| {
            c(&|a| {
                if let Some(path) = a.path(id.as_str()) {
                    a.actions.send(Command::Enabled { path, on });
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_junction_volume(move |id, v| {
            c(&|a| {
                if let Some(path) = a.path(id.as_str()) {
                    let percent = u16::try_from(v).unwrap_or(0);
                    a.later(format!("j/{id}"), Command::Volume { path, percent });
                }
            });
        });
    }
    {
        let (c, width) = (call.clone(), width);
        view.on_relayout(move |w| {
            width.set(w);
            c(&App::relayout);
        });
    }
    {
        let c = call.clone();
        view.on_card_toggled(move |column, i, on| {
            c(&|a| {
                if let Some((junction, part)) = a.part(column, i) {
                    a.actions.send(Command::Edit {
                        junction,
                        edit: Edit::Enabled(part, on),
                    });
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_card_volume(move |column, i, v| {
            c(&|a| {
                if let Some((junction, part)) = a.part(column, i) {
                    let key = format!("{junction}/{part:?}");
                    let edit = Edit::Volume(part, u16::try_from(v).unwrap_or(0));
                    a.later(key, Command::Edit { junction, edit });
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_pointer_moved(move |x, y| c(&|a| a.hover(Some(Point { x, y }))));
    }
    {
        let c = call.clone();
        view.on_pointer_left(move || c(&|a| a.hover(None)));
    }
    {
        let c = call.clone();
        view.on_link_clicked(move || {
            c(&|a| {
                if let Some(i) = a.hovered {
                    a.unlink(&[i]);
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_port_remove(move |column, card, ch, side| {
            c(&|a| a.remove_port(column, card, ch, side));
        });
    }
    {
        let c = call.clone();
        view.on_recheck(move || c(&App::recheck));
    }
    {
        let c = call.clone();
        view.on_copy(move |t| c(&|a| a.actions.send(Command::Copy(t.to_string()))));
    }
    {
        let c = call.clone();
        view.on_onboarding_continue(move || {
            c(&|a| {
                if checks::passed(&a.checks) {
                    a.set_step(Some(Step::Templates));
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_onboarding_back(move || c(&|a| a.set_step(Some(Step::Check))));
    }
    {
        let c = call.clone();
        view.on_template_chosen(move |id| c(&|a| a.template(id.as_str())));
    }
    {
        let c = call.clone();
        view.on_new_junction(move || c(&App::new_junction));
    }
    {
        let c = call.clone();
        view.on_open_picker(move || c(&App::open_picker));
        let c = call.clone();
        view.on_open_listen_picker(move || c(&App::open_listen_picker));
        let c = call.clone();
        view.on_add_bus(move |n| {
            c(&|a| {
                if let (Some(m), Ok(n)) = (&a.main, u8::try_from(n)) {
                    a.actions.send(Command::Edit {
                        junction: m.id.clone(),
                        edit: Edit::AddBus(n),
                    });
                }
            });
        });
    }
    {
        let c = call.clone();
        view.on_picker_search_edited(move |t| c(&|a| a.search(t.as_str())));
    }
    {
        let c = call.clone();
        view.on_picker_picked(move |i| c(&|a| a.pick(i)));
    }
    {
        let c = call.clone();
        view.on_picker_closed(move || c(&App::close_picker));
    }
    {
        let c = call;
        view.on_delete_junction(move || {
            c(&|a| {
                if let Some(path) = a.selected.clone().and_then(|id| a.path(&id)) {
                    a.actions.send(Command::Delete { path });
                }
            });
        });
    }
    app
}

impl App {
    /// juncd's new snapshot, or `None` while it is away.
    pub fn update(&mut self, describe: Option<Describe>) {
        self.sidebar = state::snapshot(describe.as_ref());
        self.describe = describe;
        let ids: Vec<&str> = self
            .sidebar
            .junctions
            .iter()
            .map(|r| r.id.as_str())
            .collect();
        // A New Junction selects itself once juncd shows it.
        if let Some(before) = &self.creating
            && let Some(new) = ids.iter().find(|id| !before.iter().any(|b| b == *id))
        {
            self.selected = Some((*new).to_owned());
            self.creating = None;
        }
        if self.selected.as_deref().is_none_or(|s| !ids.contains(&s)) {
            self.selected = ids.first().map(|s| (*s).to_owned());
        }
        if let Some(file) = &self.state_file {
            let done = onboarding::done(file);
            let needed = onboarding::needed(done, self.describe.as_ref());
            if self.onboarding.is_none() && needed {
                self.set_step(Some(Step::Check));
            } else if self.onboarding.is_some() && !needed {
                // Set up meanwhile (the CLI, an agent): nothing to onboard.
                self.finish_onboarding();
            }
        }
        self.refresh();
    }

    /// Onboards on a first launch, remembering it in `file`
    /// ([`onboarding::state_file`]); decided with juncd's next snapshot.
    pub fn onboard_with(&mut self, file: PathBuf) {
        self.state_file = Some(file);
    }

    /// Shows onboarding at `step` (scenarios), or no onboarding.
    pub fn set_step(&mut self, step: Option<Step>) {
        let first = self.onboarding.is_none() && step == Some(Step::Check);
        self.onboarding = step;
        if let Some(w) = self.window.upgrade() {
            w.global::<View>().set_onboarding(match step {
                None => 0,
                Some(Step::Check) => 1,
                Some(Step::Templates) => 2,
            });
        }
        if first {
            self.recheck();
        }
    }

    /// The onboarding step on screen.
    pub fn step(&self) -> Option<Step> {
        self.onboarding
    }

    fn finish_onboarding(&mut self) {
        if let Some(file) = &self.state_file
            && let Err(e) = onboarding::mark_done(file)
        {
            eprintln!("junc-gui: {}: {e}", file.display());
        }
        self.set_step(None);
    }

    fn recheck(&mut self) {
        if let Some(w) = self.window.upgrade() {
            w.global::<View>().set_checking(true);
        }
        self.actions.send(Command::Check);
    }

    /// The system check's answer.
    pub fn set_checks(&mut self, rows: Vec<checks::Row>) {
        if let Some(w) = self.window.upgrade() {
            let view = w.global::<View>();
            view.set_checking(false);
            view.set_checks_passed(checks::passed(&rows));
            view.set_checks(ModelRc::new(VecModel::from(ui::checks(&rows))));
        }
        self.checks = rows;
    }

    /// A starting point from onboarding. "Start empty" is a New Junction;
    /// templates with slots ask for them first (board 49, B22).
    fn template(&mut self, id: &str) {
        if id == "start-empty" {
            self.new_junction();
            self.finish_onboarding();
        }
    }

    fn new_junction(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let name = w.global::<Words>().invoke_new_junction().to_string();
        self.creating = Some(
            self.sidebar
                .junctions
                .iter()
                .map(|r| r.id.clone())
                .collect(),
        );
        self.actions.send(Command::Create { name });
    }

    /// Opens the Add Input picker on the selected Junction.
    pub fn open_picker(&mut self) {
        self.open(false);
    }

    /// Opens the Add Bus channels popover on the selected Junction.
    pub fn open_add_bus(&self) {
        if let (Some(_), Some(w)) = (&self.main, self.window.upgrade()) {
            w.global::<View>().set_adding_bus(true);
        }
    }

    /// Opens the Add Listen output picker on the selected Junction.
    pub fn open_listen_picker(&mut self) {
        self.open(true);
    }

    fn open(&mut self, listen: bool) {
        if self.main.is_none() {
            return;
        }
        self.picker = Some((Vec::new(), Vec::new()));
        self.picker_listen = listen;
        if let Some(w) = self.window.upgrade() {
            let view = w.global::<View>();
            view.set_picker_search("".into());
            view.set_picker_cursor(0);
            view.set_picker_listen(listen);
        }
        self.load_picker();
        if let Some(w) = self.window.upgrade() {
            w.global::<View>().set_picker_open(true);
        }
    }

    /// The picker's rows from the latest snapshot, filtered by the search.
    fn load_picker(&mut self) {
        let (Some(d), Some(id), Some(w)) = (&self.describe, &self.selected, self.window.upgrade())
        else {
            return;
        };
        let view = w.global::<View>();
        let entries = if self.picker_listen {
            picker::outputs(d, id)
        } else {
            picker::entries(d, id)
        };
        let shown = picker::filter(&entries, view.get_picker_search().as_str());
        let items = ui::picker(&w.global::<Words>(), &entries, &shown);
        view.set_picker_items(ModelRc::new(VecModel::from(items)));
        self.picker = Some((entries, shown));
    }

    fn search(&mut self, _text: &str) {
        self.load_picker();
    }

    fn pick(&mut self, i: i32) {
        let (Some((entries, shown)), Some(m)) = (&self.picker, &self.main) else {
            return;
        };
        let Some(e) = usize::try_from(i)
            .ok()
            .and_then(|i| shown.get(i))
            .and_then(|&i| entries.get(i))
        else {
            return;
        };
        if e.added {
            return;
        }
        let edit = match &e.pick {
            Pick::Input(matcher) => Edit::AddInput {
                matcher: Box::new(matcher.clone()),
                label: e.label.clone(),
            },
            Pick::Listen(target) => Edit::AddListen {
                target: target.clone(),
                label: if e.label.is_empty() {
                    target.clone()
                } else {
                    e.label.clone()
                },
            },
        };
        self.actions.send(Command::Edit {
            junction: m.id.clone(),
            edit,
        });
        self.close_picker();
    }

    fn close_picker(&mut self) {
        self.picker = None;
        if let Some(w) = self.window.upgrade() {
            w.global::<View>().set_picker_open(false);
        }
    }

    /// The selected Junction.
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// Selects Junction `id`.
    pub fn select(&mut self, id: &str) {
        if self.selected.as_deref() != Some(id) {
            self.selected = Some(id.to_owned());
            self.hovered = None;
            self.refresh();
        }
    }

    fn path(&self, id: &str) -> Option<String> {
        self.sidebar
            .junctions
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.path.clone())
    }

    /// The Input or Listen output of card `i` in `column`.
    fn part(&self, column: i32, i: i32) -> Option<(String, Part)> {
        let m = self.main.as_ref()?;
        let i = usize::try_from(i).ok()?;
        let part = match column {
            0 => Part::Input(m.inputs.get(i)?.id.clone()),
            2 => Part::Listen(m.listens.get(i)?.id.clone()),
            _ => return None,
        };
        Some((m.id.clone(), part))
    }

    /// Sends `command` once its slider has rested; the newest value per
    /// key wins, and each change postpones the send.
    fn later(&mut self, key: String, command: Command) {
        self.pending.borrow_mut().insert(key, command);
        let (actions, pending) = (self.actions.clone(), self.pending.clone());
        self.timer
            .start(slint::TimerMode::SingleShot, SETTLE, move || {
                for (_, c) in std::mem::take(&mut *pending.borrow_mut()) {
                    actions.send(c);
                }
            });
    }

    fn relayout(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let view = w.global::<View>();
        let Some(m) = &self.main else { return };
        let l = layout::layout(m, self.width.get(), view.get_show_listen());
        let rows = &self.sidebar.junctions;
        let c = ui::canvas(m, &l, self.hovered, rows.len());
        view.set_stacked(l.stacked);
        view.set_col_x0(l.columns[0].0);
        view.set_col_x1(l.columns[1].0);
        view.set_col_x2(l.columns[2].0);
        view.set_col_w(l.columns[0].1);
        view.set_canvas_height(l.height);
        view.set_listen_heading_y(l.listen_heading.unwrap_or(0.0));
        view.set_empty_inputs(l.empty_inputs.is_some());
        view.set_empty_listens(l.empty_listens.is_some());
        view.set_empty_listens_y(l.empty_listens.map_or(0.0, |r| r.y));
        view.set_empty_h(l.empty_inputs.or(l.empty_listens).map_or(0.0, |r| r.h));
        view.set_cards(ModelRc::new(VecModel::from(c.cards)));
        view.set_ports(ModelRc::new(VecModel::from(c.ports)));
        view.set_links(ModelRc::new(VecModel::from(c.links)));
        self.layout = Some(l);
    }

    /// Shows the sidebar and the selected Junction.
    fn refresh(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let view = w.global::<View>();
        let selected = self.selected.clone();
        self.main = self
            .describe
            .as_ref()
            .zip(selected.as_deref())
            .and_then(|(d, id)| main_view::main(d, id));
        let rows = &self.sidebar.junctions;
        view.set_daemon_state(self.sidebar.daemon.as_str().into());
        view.set_junctions(ModelRc::new(VecModel::from(ui::sidebar(
            rows,
            selected.as_deref(),
        ))));
        let slots = ui::slots(rows, self.main.as_ref());
        self.levels.reset(slots.len());
        if let Some(meters) = &self.meters {
            meters.set_slots(&slots);
        }
        match &self.main {
            Some(m) => {
                view.set_title(m.name.as_str().into());
                view.set_junction_on(m.enabled);
                view.set_node(m.node.as_str().into());
                view.set_bus_channels(
                    i32::try_from(m.bus.iter().map(|g| g.channels.len()).sum::<usize>())
                        .unwrap_or(0),
                );
                view.set_apps(i32::try_from(m.apps).unwrap_or(0));
                view.set_devices(i32::try_from(m.devices).unwrap_or(0));
                view.set_off(i32::try_from(m.off).unwrap_or(0));
                view.set_offline(i32::try_from(m.offline).unwrap_or(0));
                view.set_listen_count(i32::try_from(m.listens.len()).unwrap_or(0));
                view.set_selected(m.id.as_str().into());
            }
            None => {
                view.set_selected("".into());
                if self.picker.is_some() {
                    self.picker = None;
                    view.set_picker_open(false);
                }
            }
        }
        if self.picker.is_some() && self.main.is_some() {
            self.load_picker();
        }
        if self
            .hovered
            .is_some_and(|i| self.main.as_ref().is_none_or(|m| i >= m.links.len()))
        {
            self.hovered = None;
            view.set_tip_visible(false);
        }
        self.relayout();
    }

    fn hover(&mut self, p: Option<Point>) {
        let hit = p.and_then(|p| self.layout.as_ref()?.hit(p));
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let view = w.global::<View>();
        if let (Some(i), Some(p), Some(m)) = (hit, p, &self.main) {
            let link = &m.links[i];
            let (card, ch, bus, listen) = match (link.from, link.to) {
                (End::Input { card, ch }, End::BusIn { ch: bus }) => {
                    (&m.inputs[card], ch, bus, false)
                }
                (End::BusOut { ch: bus }, End::Listen { card, ch }) => {
                    (&m.listens[card], ch, bus, true)
                }
                _ => return,
            };
            let pos = card
                .channels
                .iter()
                .find(|c| c.number == ch)
                .map_or("", |c| c.position);
            view.set_tip_owner(card.title.as_str().into());
            view.set_tip_ch(i32::from(ch));
            view.set_tip_pos(pos.into());
            view.set_tip_bus(i32::from(bus));
            view.set_tip_listen(listen);
            view.set_tip_x(p.x);
            view.set_tip_y(p.y);
        }
        view.set_tip_visible(hit.is_some());
        if hit != self.hovered {
            self.hovered = hit;
            self.relayout();
        }
    }

    /// Sets the link under the pointer (scenarios).
    pub fn hover_at(&mut self, p: Point) {
        self.hover(Some(p));
    }

    /// The layout on screen.
    pub fn layout(&self) -> Option<&Layout> {
        self.layout.as_ref()
    }

    /// The Main model on screen.
    pub fn main(&self) -> Option<&Main> {
        self.main.as_ref()
    }

    /// Removes links `links` (indexes into the Main model's links).
    fn unlink(&mut self, links: &[usize]) {
        let (Some(m), Some(d)) = (&self.main, &self.describe) else {
            return;
        };
        let Some(j) = d.junctions.iter().find(|j| j.id == m.id) else {
            return;
        };
        let mut by_owner: BTreeMap<(bool, String), Vec<[u8; 2]>> = BTreeMap::new();
        for &i in links {
            let Some(l) = m.links.get(i) else { continue };
            let listen = matches!(l.to, End::Listen { .. });
            by_owner
                .entry((listen, l.owner.clone()))
                .or_default()
                .push(l.pair);
        }
        for ((listen, owner), remove) in by_owner {
            let (part, in_use) = if listen {
                let Some(v) = j.listens.iter().find(|v| v.id == owner) else {
                    continue;
                };
                (Part::Listen(owner), v.links.clone())
            } else {
                let Some(v) = j.inputs.iter().find(|v| v.id == owner) else {
                    continue;
                };
                (Part::Input(owner), v.links.clone())
            };
            self.actions.send(Command::Edit {
                junction: m.id.clone(),
                edit: Edit::Unlink {
                    part,
                    remove,
                    in_use,
                },
            });
        }
    }

    /// Del on a port: removes every link at it.
    fn remove_port(&mut self, column: i32, card: i32, ch: i32, side: i32) {
        let Some(m) = &self.main else { return };
        let (Ok(card), Ok(ch)) = (usize::try_from(card), u8::try_from(ch)) else {
            return;
        };
        let end = match (column, side) {
            (0, _) => End::Input { card, ch },
            (1, 0) => End::BusIn { ch },
            (1, _) => End::BusOut { ch },
            _ => End::Listen { card, ch },
        };
        let links: Vec<usize> = m
            .links
            .iter()
            .enumerate()
            .filter(|(_, l)| l.from == end || l.to == end)
            .map(|(i, _)| i)
            .collect();
        self.unlink(&links);
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

/// Makes `app` the one [`update_current`] reaches (GUI thread).
pub fn set_current(app: &Rc<RefCell<App>>) {
    CURRENT.with(|c| *c.borrow_mut() = Some(app.clone()));
}

/// Gives the current controller juncd's new snapshot; call it on the GUI
/// thread (`slint::invoke_from_event_loop` from others).
pub fn update_current(describe: Option<Describe>) {
    let app = CURRENT.with(|c| c.borrow().clone());
    if let Some(a) = app {
        a.borrow_mut().update(describe);
    }
}

/// Gives the current controller the system check's answer (GUI thread).
pub fn checks_current(rows: Vec<checks::Row>) {
    let app = CURRENT.with(|c| c.borrow().clone());
    if let Some(a) = app {
        a.borrow_mut().set_checks(rows);
    }
}

/// Shows the current controller again, after a language change.
pub fn refresh_current() {
    let app = CURRENT.with(|c| c.borrow().clone());
    if let Some(a) = app
        && let Ok(mut a) = a.try_borrow_mut()
    {
        a.refresh();
    }
}
