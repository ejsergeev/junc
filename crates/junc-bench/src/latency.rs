// SPDX-License-Identifier: GPL-3.0-or-later

//! Added latency through a Junction, the way the validation measured it
//! (bench/validation/bench/impulse_latency.py): synchronous JACK clients on
//! the graph clock. A sender emits a 32-sample burst every half second; a
//! receiver hears it directly on one port and through the Junction on the
//! other. The difference of their first loud frames is what the Junction
//! adds. (Timing `pw-cat` streams instead mixes in where the graph orders
//! those asynchronous clients, which is not the Junction's latency.)

use jack::contrib::ClosureProcessHandler;
use jack::{AudioIn, AudioOut, Client, ClientOptions, Control};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PERIOD: u32 = 24_000;
const BURST: usize = 32;

fn link(from: &str, to: &str) -> Result<(), String> {
    for _ in 0..300 {
        let ok = Command::new("pw-link")
            .args([from, to])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(format!("cannot link {from} → {to}"))
}

/// Median frames the path `into` → `out_of` adds compared with a direct
/// connection, and the quantum. `into` is a Junction input port (a gain
/// node's playback port), `out_of` the Junction's output port.
pub fn added_frames(into: &str, out_of: &str, secs: u64) -> Result<(f64, u32), String> {
    let jerr = |e: jack::Error| format!("JACK: {e}");
    let (tx, _) = Client::new("junc_bench_tx", ClientOptions::NO_START_SERVER).map_err(jerr)?;
    let (rx, _) = Client::new("junc_bench_rx", ClientOptions::NO_START_SERVER).map_err(jerr)?;
    let mut out = tx.register_port("out", AudioOut::default()).map_err(jerr)?;
    let direct = rx
        .register_port("direct", AudioIn::default())
        .map_err(jerr)?;
    let via = rx
        .register_port("junction", AudioIn::default())
        .map_err(jerr)?;
    let quantum = tx.buffer_size();

    let send = ClosureProcessHandler::new(move |_: &Client, ps: &jack::ProcessScope| {
        let o = out.as_mut_slice(ps);
        o.fill(0.0);
        let k = (PERIOD - ps.last_frame_time() % PERIOD) % PERIOD;
        let k = k as usize;
        if k < o.len() {
            let end = (k + BURST).min(o.len());
            o[k..end].fill(0.5);
        }
        Control::Continue
    });
    // (port, frame on the graph clock) of the first loud sample of each block.
    let heard: Arc<Mutex<Vec<(u8, u32)>>> = Arc::default();
    let sink = heard.clone();
    let recv = ClosureProcessHandler::new(move |_: &Client, ps: &jack::ProcessScope| {
        let f0 = ps.last_frame_time();
        for (port, data) in [(0u8, direct.as_slice(ps)), (1, via.as_slice(ps))] {
            if let Some(i) = data.iter().position(|s| s.abs() > 0.05)
                && let Ok(mut v) = sink.try_lock()
            {
                #[allow(clippy::cast_possible_truncation)]
                v.push((port, f0.wrapping_add(i as u32)));
            }
        }
        Control::Continue
    });
    let tx = tx.activate_async((), send).map_err(jerr)?;
    let rx = rx.activate_async((), recv).map_err(jerr)?;
    link("junc_bench_tx:out", "junc_bench_rx:direct")?;
    link("junc_bench_tx:out", into)?;
    link(out_of, "junc_bench_rx:junction")?;
    std::thread::sleep(Duration::from_secs(secs));
    drop(rx.deactivate());
    drop(tx.deactivate());

    // The first frame per port in each burst period, paired up.
    let heard = heard.lock().map_err(|_| "poisoned")?.clone();
    let first = |port: u8| -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = Vec::new();
        for &(p, f) in &heard {
            let burst = f / PERIOD;
            if p == port && out.last().is_none_or(|(b, _)| *b != burst) {
                out.push((burst, f));
            }
        }
        out
    };
    let (d, j) = (first(0), first(1));
    let mut diffs: Vec<f64> = d
        .iter()
        .filter_map(|(b, fd)| {
            j.iter()
                .find(|(bj, _)| bj == b)
                .map(|(_, fj)| f64::from(*fj) - f64::from(*fd))
        })
        .collect();
    if diffs.len() < 3 {
        return Err(format!(
            "only {} bursts arrived on both paths ({} direct, {} through the Junction)",
            diffs.len(),
            d.len(),
            j.len()
        ));
    }
    diffs.sort_by(f64::total_cmp);
    Ok((diffs[diffs.len() / 2], quantum))
}
