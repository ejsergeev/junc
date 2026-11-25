// JUNC throwaway Slint prototype.
// Flags:
//   --snapshot <file.png>   render once, write PNG, exit (for i18n screenshots)
//   --cold-start            print ms from process start to first frame, exit
//   --no-meters             disable the 7 Hz meter timer
// Env:
//   JUNC_LANG=<code>        force initial language (else ~/.config/omarchy/language, else "en")
use slint::{ComponentHandle, Model, ModelRc, SharedString, StandardListViewItem, VecModel};
use std::{path::PathBuf, rc::Rc, time::{Duration, Instant}};

slint::include_modules!();

const RTL: &[&str] = &["ar", "he", "fa", "ur"];

fn omarchy_lang_file() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/omarchy/language")
}

fn read_lang() -> String {
    if let Ok(l) = std::env::var("JUNC_LANG") { return l; }
    std::fs::read_to_string(omarchy_lang_file())
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_owned))
        .unwrap_or_else(|| "en".into())
}

fn apply_lang(ui: &MainWindow, lang: &str) {
    // gettext picks the catalog from LANGUAGE; locale (LANG) must be non-"C" for that to work.
    // Normalise "zh-CN" -> "zh_CN" (gettext uses underscores).
    let gl = lang.replace('-', "_");
    unsafe { std::env::set_var("LANGUAGE", &gl) };
    slint::update_all_translations();
    let base = gl.split('_').next().unwrap_or("");
    ui.set_rtl(RTL.contains(&base));
    ui.set_lang(lang.into());
    eprintln!("[junc-proto] language={lang} rtl={}", RTL.contains(&base));
}

fn main() {
    let t0 = Instant::now();
    let args: Vec<String> = std::env::args().collect();
    let snapshot = args.iter().position(|a| a == "--snapshot").map(|i| args[i + 1].clone());
    let cold = args.iter().any(|a| a == "--cold-start");
    let meters = !args.iter().any(|a| a == "--no-meters");

    let catalog_dir = std::env::var("JUNC_LOCALEDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/locale")));
    slint::init_translations!(catalog_dir);

    let ui = MainWindow::new().unwrap();
    apply_lang(&ui, &read_lang());
    // JUNC_WIDTH=<px>: start narrower (overflow / line-breaking tests)
    if let Some(w) = std::env::var("JUNC_WIDTH").ok().and_then(|w| w.parse::<f32>().ok()) {
        ui.window().set_size(slint::LogicalSize::new(w, 640.0));
    }

    // --plural-dump: print the plural string for a range of n (checks gettext plural forms via Slint)
    if args.iter().any(|a| a == "--plural-dump") {
        for n in [0, 1, 2, 3, 4, 5, 10, 11, 12, 21, 22, 25, 100, 101, 102, 111] {
            ui.set_connected_count(n);
            println!("{n}\t{}", ui.get_count_text());
        }
        return;
    }

    // ---- picker model + filtering ----
    let all: Vec<&str> = vec!["Firefox", "Spotify", "Discord", "mpv", "OBS Studio", "Chromium",
        "USB Mic (Scarlett 2i2)", "Built-in Microphone", "HDMI Output", "Steam"];
    let items = Rc::new(VecModel::from(all.iter().map(|s| StandardListViewItem::from(*s)).collect::<Vec<_>>()));
    ui.set_picker_items(ModelRc::from(items.clone()));
    {
        let items = items.clone();
        ui.on_picker_query_changed(move |q: SharedString| {
            let q = q.to_lowercase();
            items.set_vec(all.iter().filter(|s| s.to_lowercase().contains(&q))
                .map(|s| StandardListViewItem::from(*s)).collect::<Vec<_>>());
        });
    }
    {
        let items = items.clone();
        ui.on_picker_chosen(move |i| {
            if let Some(it) = items.row_data(i as usize) { eprintln!("[junc-proto] picked {}", it.text); }
        });
    }

    // ---- live language switch: inotify on ~/.config/omarchy (no polling => no idle wakeups) ----
    {
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            use inotify::{Inotify, WatchMask};
            let f = omarchy_lang_file();
            let dir = f.parent().unwrap().to_owned();
            let _ = std::fs::create_dir_all(&dir);
            let mut ino = Inotify::init().expect("inotify");
            ino.watches().add(&dir, WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO | WatchMask::CREATE).unwrap();
            let mut buf = [0u8; 4096];
            loop {
                let evs = ino.read_events_blocking(&mut buf).unwrap();
                if evs.into_iter().any(|e| e.name.map(|n| n == "language").unwrap_or(false)) {
                    let w = weak.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = w.upgrade() { apply_lang(&ui, &read_lang()); }
                    });
                }
            }
        });
    }

    // ---- meters at 7 updates/s ----
    let timer = slint::Timer::default();
    if meters && snapshot.is_none() {
        let weak = ui.as_weak();
        let mut seed: u32 = 12345;
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(1000 / 7), move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut rnd = || { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); (seed >> 8) as f32 / (1 << 24) as f32 };
            let m = ui.get_inputs();
            for i in 0..m.row_count() { let mut c = m.row_data(i).unwrap(); if c.enabled { c.level = rnd(); } m.set_row_data(i, c); }
            let o = ui.get_outputs();
            for i in 0..o.row_count() { let mut c = o.row_data(i).unwrap(); c.level = rnd(); o.set_row_data(i, c); }
            let mut b = ui.get_bus(); b.level = rnd(); ui.set_bus(b);
        });
    }

    // ---- cold-start / snapshot hooks ----
    // Rendering notifier gives "first frame" timing on GL renderers; the software renderer does not
    // support it, so snapshots fall back to a timer once the event loop is running.
    let notifier_ok = if cold {
        let fired = std::cell::Cell::new(false);
        ui.window().set_rendering_notifier(move |state, _| {
            if let slint::RenderingState::AfterRendering = state {
                if !fired.replace(true) {
                    println!("first_frame_ms={}", t0.elapsed().as_millis());
                    let _ = slint::quit_event_loop();
                }
            }
        }).is_ok()
    } else { false };
    if cold && !notifier_ok {
        // fallback: first event-loop turn after show() (frame has been requested, maybe not presented)
        slint::Timer::single_shot(Duration::from_millis(0), move || {
            println!("first_loop_turn_ms={} (renderer has no rendering notifier)", t0.elapsed().as_millis());
            let _ = slint::quit_event_loop();
        });
    }
    if let Some(p) = snapshot.clone() {
        let w = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(700), move || {
            if let Some(ui) = w.upgrade() {
                let buf = ui.window().take_snapshot().expect("snapshot");
                let file = std::fs::File::create(&p).unwrap();
                let mut enc = png::Encoder::new(std::io::BufWriter::new(file), buf.width(), buf.height());
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header().unwrap().write_image_data(buf.as_bytes()).unwrap();
            }
            let _ = slint::quit_event_loop();
        });
    }

    ui.run().unwrap();
}
