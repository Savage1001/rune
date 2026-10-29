//! Retained mounted subtrees (t726): the same headless trees under each `SubtreeRetention`
//! mode (`FullRebuild`, `BuilderMemo`, `RetainedMounted`), paired in one process, alternating
//! which mode runs first each round. No window, renderer or GPU.
//!
//! `rebuild`: a settled 250-card tree (2,252 mounted nodes, about Seer's 2,230) where one card
//! and the header counter update while 249 mounted cards stay unchanged.
//! `burst`: the same tree with a spinner and one card's text growing every frame for 120 frames.
//! `churn`: every card's version changes on every commit (0% subtree reuse, worst-case check).
//! `swap`: 250 cards, then 5, then 250, five times, settling exits each time.
//!
//! cargo build --profile perf -p rune --example retained_subtrees --features frame-profile
//! target/perf/examples/retained_subtrees [--samples 41] [--only rebuild|burst|churn|swap]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicI64, Ordering::Relaxed};
use std::time::Instant;

use rune::{Cx, El, Env, LoopSpec, Runtime, SubtreeRetention, col, el, memo, oklch, row, text};

struct Counting;
static LIVE: AtomicI64 = AtomicI64::new(0);
static PEAK: AtomicI64 = AtomicI64::new(0);
thread_local! {
    static BYTES: Cell<u64> = const { Cell::new(0) };
    static CALLS: Cell<u64> = const { Cell::new(0) };
}
fn grew(n: usize) {
    let live = LIVE.fetch_add(n as i64, Relaxed) + n as i64;
    PEAK.fetch_max(live, Relaxed);
    let _ = BYTES.try_with(|b| b.set(b.get() + n as u64));
    let _ = CALLS.try_with(|c| c.set(c.get() + 1));
}
// SAFETY: every call goes straight to System; the counters never allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            grew(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size() as i64, Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            LIVE.fetch_sub(l.size() as i64, Relaxed);
            grew(n);
        }
        q
    }
}
#[global_allocator]
static ALLOC: Counting = Counting;

fn counters() -> (u64, u64) {
    (BYTES.get(), CALLS.get())
}
fn live() -> i64 {
    LIVE.load(Relaxed)
}

struct Card {
    id: usize,
    rev: u64,
    title: String,
    summary: String,
    tag: String,
}

struct State {
    cards: Vec<Card>,
    visible: usize,
    spinner: bool,
    tick: u64,
    global_rev: u64,
    tail: String,
}

fn card_view(id: usize, rev: u64, title: String, summary: String, tag: String) -> El {
    col()
        .w_full()
        .padding(8.0)
        .gap(4.0)
        .radius(6.0)
        .fill(oklch(0.24, 0.02, 250.0))
        .child(
            row()
                .key("hdr")
                .w_full()
                .gap(6.0)
                .child(el().key("dot").size(10.0, 10.0).radius(5.0).fill(oklch(0.68, 0.14, 250.0)))
                .child(text(title).key("title").grow(1.0))
                .child(text(tag).key("tag")),
        )
        .child(
            col()
                .key("body")
                .w_full()
                .gap(2.0)
                .child(text(summary).key("sum").wrap())
                .child(text(format!("card #{id} · r{rev}")).key("meta")),
        )
        .child(el().key("act").size(16.0, 16.0).on_press(|_: &mut State| {}))
}

fn view(s: &State, _: &mut Cx<State>) -> El {
    let mut root = col()
        .key("root")
        .w_full()
        .gap(4.0)
        .child(text(format!("tick {}", s.tick)).key("tick"));
    if s.spinner {
        root = root.child(el().key("spinner").size(12.0, 12.0).looping(LoopSpec::rotation(1.0)));
    }
    let last = s.visible.saturating_sub(1);
    root.children((0..s.visible).map(|i| {
        let c = &s.cards[i];
        let id = c.id;
        let rev = c.rev ^ s.global_rev ^ if i == last { s.tick } else { 0 };
        memo(format!("c{id}"), rev, || {
            let summary = if i == last { s.tail.clone() } else { c.summary.clone() };
            card_view(id, rev, c.title.clone(), summary, c.tag.clone())
        })
    }))
}

const MODES: [(SubtreeRetention, &str); 3] = [
    (SubtreeRetention::FullRebuild, "full-rebuild"),
    (SubtreeRetention::BuilderMemo, "builder-memo"),
    (SubtreeRetention::RetainedMounted, "retained-mounted"),
];

fn app(mode: SubtreeRetention, visible: usize, spinner: bool) -> Runtime<State> {
    let cards = (0..300)
        .map(|i| Card {
            id: i,
            rev: 1,
            title: format!("Task t{}: retain unchanged mounted subtrees", 700 + i),
            summary: format!("Row {i}: benchmark card summary with wrapped description text."),
            tag: format!("p{}", i % 4),
        })
        .collect();
    let state = State {
        cards,
        visible,
        spinner: false,
        tick: 0,
        global_rev: 0,
        tail: "Streaming".into(),
    };
    let mut rt = Runtime::new(state, view, (900.0, 24000.0), Env::new(true, 2.0));
    rt.set_subtree_retention(mode);
    rt.frame(0.0);
    settle(&mut rt);
    if spinner {
        rt.state.spinner = true;
        rt.invalidate();
        rt.frame(1.0 / 120.0);
    }
    rt
}

fn settle(rt: &mut Runtime<State>) -> usize {
    let mut frames = 0;
    while frames < 2000 && rt.frame(1.0 / 120.0) {
        frames += 1;
    }
    frames
}

/// One committing frame: [bytes, calls, wall ms, thread cpu ms].
fn timed(rt: &mut Runtime<State>) -> [f64; 4] {
    let (b0, c0) = counters();
    let cpu = rune::thread_cpu_ms();
    let at = Instant::now();
    rt.frame(1.0 / 120.0);
    let wall = at.elapsed().as_secs_f64() * 1000.0;
    let cpu = rune::thread_cpu_ms() - cpu;
    let (b1, c1) = counters();
    [(b1 - b0) as f64, (c1 - c0) as f64, wall, cpu]
}

fn report(scenario: &str, mode: &str, metric: &str, mut v: Vec<f64>) {
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    println!(
        "METRIC scenario={scenario} mode={mode} metric={metric} p50={:.4} p95={:.4} min={:.4} max={:.4} n={}",
        at(0.5),
        at(0.95),
        v[0],
        v[v.len() - 1],
        v.len()
    );
}

fn order(round: usize) -> [usize; 3] {
    [[0, 1, 2], [2, 1, 0], [1, 2, 0], [0, 2, 1], [2, 0, 1], [1, 0, 2]][round % 6]
}

const FRAME: [&str; 4] = ["bytes", "calls", "wall_ms", "cpu_ms"];

fn rebuild(samples: usize) {
    let mut rts: Vec<_> = MODES.iter().map(|&(m, _)| app(m, 250, false)).collect();
    let mut rows = vec![Vec::new(); 3];
    for round in 0..=samples {
        for i in order(round) {
            let rt = &mut rts[i];
            rt.state.tick = round as u64 + 1;
            rt.state.tail = format!("Tail {round}");
            rt.invalidate();
            let r = timed(rt);
            settle(rt);
            if round > 0 {
                rows[i].push(r);
            }
        }
    }
    let shots: Vec<_> = rts
        .iter()
        .map(|rt| {
            rt.elements()
                .into_iter()
                .map(|e| (e.path, e.rect, e.text))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(shots.iter().all(|s| *s == shots[0]), "modes laid out differently");
    let inst_bytes: Vec<&[u8]> = rts.iter().map(|rt| bytemuck::cast_slice(&rt.instances)).collect();
    assert!(
        inst_bytes.iter().all(|b| *b == inst_bytes[0]),
        "modes painted differently"
    );
    for (i, &(_, name)) in MODES.iter().enumerate() {
        for (m, metric) in FRAME.iter().enumerate() {
            report("rebuild", name, metric, rows[i].iter().map(|r| r[m]).collect());
        }
        let st = rts[i].retained_subtree_stats();
        println!(
            "METRIC scenario=rebuild mode={name} nodes={} retained_subtrees={} spliced_subtrees={} spliced_nodes={}",
            rts[i].stats.layout_nodes, st.retained_subtrees, st.spliced_subtrees, st.spliced_nodes
        );
    }
}

fn settled(samples: usize) {
    let mut out = vec![Vec::new(); 3];
    for round in 0..=samples.div_ceil(6).max(3) {
        for i in order(round) {
            let base = live();
            let mut rt = app(MODES[i].0, 250, false);
            for k in 0..8 {
                rt.state.tick = k + 1;
                rt.state.tail = format!("Tail {k}");
                rt.invalidate();
                rt.frame(1.0 / 120.0);
                settle(&mut rt);
            }
            if round > 0 {
                out[i].push((live() - base) as f64);
            }
        }
    }
    for (i, &(_, name)) in MODES.iter().enumerate() {
        report("rebuild", name, "settled", out[i].clone());
    }
}

fn burst(_samples: usize) {
    let frames = 30;
    let mut out: Vec<Vec<[f64; 4]>> = vec![Vec::new(); 3];
    let mut peaks = vec![Vec::new(); 3];
    for round in 0..=3 {
        for i in order(round) {
            let mut rt = app(MODES[i].0, 250, true);
            let base = live();
            PEAK.store(base, Relaxed);
            for f in 0..frames {
                rt.state.tick += 1;
                rt.state.tail.push_str(if f % 9 == 8 { " streaming\n" } else { " word" });
                rt.invalidate();
                let r = timed(&mut rt);
                if round > 0 {
                    out[i].push(r);
                }
            }
            if round > 0 {
                peaks[i].push((PEAK.load(Relaxed) - base) as f64);
            }
        }
    }
    for (i, &(_, name)) in MODES.iter().enumerate() {
        for (m, metric) in FRAME.iter().enumerate() {
            report("burst", name, metric, out[i].iter().map(|r| r[m]).collect());
        }
        report("burst", name, "peak_over_start", peaks[i].clone());
    }
}

fn churn(samples: usize) {
    let rounds = samples.min(21);
    let mut rts: Vec<_> = MODES.iter().map(|&(m, _)| app(m, 250, false)).collect();
    let mut rows = vec![Vec::new(); 3];
    for round in 0..=rounds {
        for i in order(round) {
            let rt = &mut rts[i];
            rt.state.tick = round as u64 + 1;
            rt.state.global_rev = round as u64 + 1;
            rt.invalidate();
            let r = timed(rt);
            settle(rt);
            if round > 0 {
                rows[i].push(r);
            }
        }
    }
    for (i, &(_, name)) in MODES.iter().enumerate() {
        for (m, metric) in FRAME.iter().enumerate() {
            report("churn", name, metric, rows[i].iter().map(|r| r[m]).collect());
        }
    }
}

fn swap(_samples: usize) {
    let mut out = vec![vec![Vec::new(); 5]; 3];
    for round in 0..=3 {
        for i in order(round) {
            let base = live();
            let mut rt = app(MODES[i].0, 250, false);
            let start = live();
            PEAK.store(start, Relaxed);
            let (b0, c0) = counters();
            let mut small_retained = 0usize;
            for _ in 0..3 {
                for vis in [5, 250] {
                    rt.state.visible = vis;
                    rt.state.tick += 1;
                    rt.invalidate();
                    rt.frame(1.0 / 120.0);
                    settle(&mut rt);
                    if vis == 5 {
                        small_retained = rt.retained_subtree_stats().retained_subtrees;
                    }
                }
            }
            let (b1, c1) = counters();
            let r = [
                (b1 - b0) as f64,
                (c1 - c0) as f64,
                (PEAK.load(Relaxed) - start) as f64,
                (live() - base) as f64,
                small_retained as f64,
            ];
            if round > 0 {
                for (m, v) in r.into_iter().enumerate() {
                    out[i][m].push(v);
                }
            }
        }
    }
    for (i, &(_, name)) in MODES.iter().enumerate() {
        for (m, metric) in ["bytes", "calls", "peak_over_start", "settled", "small_retained"]
            .iter()
            .enumerate()
        {
            report("swap", name, metric, out[i][m].clone());
        }
    }
}

fn main() {
    assert!(!cfg!(debug_assertions), "use --profile perf");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let samples: usize = value("--samples").map_or(41, |s| s.parse().expect("--samples N"));
    let only = value("--only");
    let run = |name: &str| only.as_deref().is_none_or(|o| o == name);
    // Warm up fonts and shared caches before counting.
    drop(app(SubtreeRetention::FullRebuild, 250, true));
    if run("rebuild") {
        rebuild(samples);
        settled(samples);
    }
    if run("burst") {
        burst(samples);
    }
    if run("churn") {
        churn(samples);
    }
    if run("swap") {
        swap(samples);
    }
}
