use std::cell::Cell;

use rune::{
    Cx, El, Env, GrowthSpec, Live, LiveText, Runtime, ScrollSpec, SubtreeRetention, Transition,
    col, el, memo, oklch, oklcha, row, text,
};

fn commit<S>(rt: &mut Runtime<S>) {
    rt.invalidate();
    rt.frame(1.0 / 60.0);
}

#[derive(Debug, PartialEq)]
struct ElSnapshot {
    path: String,
    key: Option<String>,
    role: &'static str,
    label: Option<String>,
    text: Option<String>,
    rect: [f32; 4],
    target: [f32; 4],
    opacity: f32,
    scale: f32,
    focused: bool,
    exiting: bool,
}

fn snapshot<S: 'static>(rt: &Runtime<S>) -> (Vec<ElSnapshot>, Vec<u8>) {
    let els = rt
        .elements()
        .into_iter()
        .map(|e| ElSnapshot {
            path: e.path,
            key: e.key,
            role: e.role,
            label: e.label,
            text: e.text,
            rect: e.rect,
            target: e.target,
            opacity: e.opacity,
            scale: e.scale,
            focused: e.focused,
            exiting: e.exiting,
        })
        .collect();
    let inst = bytemuck::cast_slice(&rt.instances).to_vec();
    (els, inst)
}

fn assert_modes_match<S: Clone + 'static>(
    init: S,
    view: fn(&S, &mut Cx<S>) -> El,
    steps: impl Fn(&mut Runtime<S>),
) {
    let run = |mode: SubtreeRetention| {
        let mut rt = Runtime::new(init.clone(), view, (800.0, 600.0), Env::new(true, 1.0));
        rt.set_subtree_retention(mode);
        rt.frame(0.0);
        steps(&mut rt);
        snapshot(&rt)
    };
    let (els_full, inst_full) = run(SubtreeRetention::FullRebuild);
    let (els_memo, inst_memo) = run(SubtreeRetention::BuilderMemo);
    let (els_ret, inst_ret) = run(SubtreeRetention::RetainedMounted);

    assert_eq!(els_full, els_memo, "FullRebuild vs BuilderMemo elements mismatch");
    assert_eq!(els_memo, els_ret, "BuilderMemo vs RetainedMounted elements mismatch");
    assert_eq!(inst_full, inst_memo, "FullRebuild vs BuilderMemo instances mismatch");
    assert_eq!(inst_memo, inst_ret, "BuilderMemo vs RetainedMounted instances mismatch");
}

#[derive(Clone)]
struct TranscriptState {
    tick: u32,
    theme_alt: bool,
    selected: usize,
    rows: Vec<(u32, u32, String)>,
}

fn transcript_view(s: &TranscriptState, _cx: &mut Cx<TranscriptState>) -> El {
    let bg = if s.theme_alt { oklch(0.24, 0.03, 250.0) } else { oklch(0.18, 0.02, 250.0) };
    col()
        .key("root")
        .fill(bg)
        .gap(6.0)
        .padding(12.0)
        .child(text(format!("tick {}", s.tick)).key("header"))
        .child(
            col()
                .key("viewport")
                .scroll(ScrollSpec { stick_bottom: true })
                .h(480.0)
                .child(
                    col()
                        .key("list")
                        .growth(GrowthSpec::default())
                        .virtualize(44.0)
                        .gap(4.0)
                        .children(s.rows.iter().map(|(id, rev, body)| {
                            let id = *id;
                            let rev = *rev;
                            let is_sel = s.selected == id as usize;
                            let body = body.clone();
                            memo(format!("row-{id}"), (rev, s.theme_alt, is_sel), move || {
                                row()
                                    .padding(8.0)
                                    .gap(8.0)
                                    .radius(6.0)
                                    .fill(if is_sel {
                                        oklch(0.42, 0.08, 250.0)
                                    } else {
                                        oklch(0.26, 0.02, 250.0)
                                    })
                                    .hover(oklcha(1.0, 0.0, 0.0, 0.08))
                                    .on_press(move |st: &mut TranscriptState| {
                                        st.selected = id as usize;
                                    })
                                    .child(el().key("badge").size(16.0, 16.0).radius(8.0).fill(oklch(0.65, 0.15, 250.0)))
                                    .child(
                                        col()
                                            .key("body")
                                            .gap(2.0)
                                            .child(text(format!("Item #{id} (v{rev})")).key("title"))
                                            .child(text(body).key("text").wrap()),
                                    )
                            })
                        })),
                ),
        )
}

#[test]
fn retained_mounted_splices_unchanged_subtrees_and_matches_baselines() {
    let rows: Vec<_> = (0..40)
        .map(|i| (i, 1, format!("Message body for row {i} with enough words to wrap cleanly.")))
        .collect();
    let init = TranscriptState { tick: 0, theme_alt: false, selected: 1, rows };

    let mut rt = Runtime::new(init.clone(), transcript_view, (800.0, 600.0), Env::new(true, 1.0));
    rt.frame(0.0);
    assert!(rt.retained_subtree_stats().retained_subtrees > 0);

    rt.state.tick += 1;
    commit(&mut rt);
    let stats = rt.retained_subtree_stats();
    assert!(stats.spliced_subtrees > 0, "expected mounted memo rows to be spliced");
    assert!(stats.spliced_nodes >= stats.spliced_subtrees * 4);

    assert_modes_match(init, transcript_view, |rt| {
        // Unrelated counter tick.
        rt.state.tick += 1;
        commit(rt);

        // Single row revision update + sibling reorder.
        rt.state.rows[2].1 += 1;
        rt.state.rows[2].2 = "Updated third message content".to_owned();
        rt.state.rows.swap(3, 5);
        commit(rt);
        rt.frame(1.0 / 60.0);

        // Insert a row at the top and delete a row in the middle.
        rt.state.rows.insert(0, (999, 1, "Inserted at top".to_owned()));
        rt.state.rows.remove(4);
        commit(rt);
        rt.frame(1.0);

        // Theme change invalidates versions.
        rt.state.theme_alt = true;
        commit(rt);

        // Dark-mode and scale changes invalidate env.
        rt.set_system_dark(false);
        rt.frame(1.0 / 60.0);
        rt.set_viewport((800.0, 600.0), 2.0);
        rt.frame(1.0 / 60.0);
    });
}

#[test]
fn press_hover_and_focus_handlers_work_inside_spliced_subtrees() {
    let rows: Vec<_> = (0..8).map(|i| (i, 1, format!("Row {i}"))).collect();
    let init = TranscriptState { tick: 0, theme_alt: false, selected: 0, rows };

    assert_modes_match(init, transcript_view, |rt| {
        // Commit once so subtrees are spliced.
        rt.state.tick += 1;
        commit(rt);

        let (cx, cy) = rt.find("row-2").expect("row-2 mounted");
        rt.pointer_move(cx, cy);
        rt.frame(1.0 / 60.0);
        rt.pointer_down(cx, cy);
        rt.pointer_up(cx, cy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.selected, 2);
    });
}

#[test]
fn nested_memos_and_layout_size_reads_match_across_modes() {
    #[derive(Clone)]
    struct NestedState {
        tick: u32,
        outer_v: u32,
        inner_v: u32,
        wide: bool,
    }

    fn nested_view(s: &NestedState, cx: &mut Cx<NestedState>) -> El {
        let outer_v = s.outer_v;
        let inner_v = s.inner_v;
        let probe_w = cx.layout_size("probe").map_or(0.0, |sz| sz[0]);
        col()
            .key("root")
            .w(if s.wide { 700.0 } else { 400.0 })
            .child(
                el().key("probe")
                    .w(if s.wide { 320.0 } else { 160.0 })
                    .h(24.0),
            )
            .child(memo("outer", (outer_v, probe_w.to_bits()), move || {
                col()
                    .key("outer-box")
                    .padding(4.0)
                    .child(text(format!("outer {outer_v} w={probe_w}")).key("outer-label"))
                    .child(memo("inner", inner_v, move || {
                        row()
                            .key("inner-box")
                            .gap(4.0)
                            .child(text(format!("inner {inner_v}")).key("inner-label"))
                    }))
            }))
            .child(text(format!("tick {}", s.tick)).key("tick"))
    }

    let init = NestedState { tick: 0, outer_v: 1, inner_v: 1, wide: false };
    assert_modes_match(init, nested_view, |rt| {
        // Spliced outer + inner.
        rt.state.tick += 1;
        commit(rt);

        // Outer rebuilds while inner stays cached and splices.
        rt.state.outer_v += 1;
        commit(rt);

        // Probe size change invalidates layout_size read.
        rt.state.wide = true;
        commit(rt);
    });
}

#[test]
fn live_bindings_and_exit_pruning_preserve_retained_subtrees() {
    #[derive(Clone)]
    struct LiveState {
        tick: u32,
        show_temp: bool,
        opacity: Live<f32>,
        label: LiveText,
    }

    fn live_view(s: &LiveState, _cx: &mut Cx<LiveState>) -> El {
        let opacity = s.opacity.clone();
        let label = s.label.clone();
        let mut root = col().key("root").gap(6.0);
        if s.show_temp {
            root = root.child(
                el().key("leaving")
                    .size(80.0, 24.0)
                    .fill(oklch(0.6, 0.18, 25.0))
                    .transition(Transition::opacity()),
            );
        }
        root.child(memo("kept", 1u32, move || {
            row()
                .padding(6.0)
                .live_opacity(&opacity)
                .child(text("00:00").key("clock").live_text(&label))
        }))
        .child(text(format!("tick {}", s.tick)).key("tick"))
    }

    let make_init = || LiveState {
        tick: 0,
        show_temp: true,
        opacity: Live::new(0.9),
        label: LiveText::new("00:01"),
    };

    let run = |mode: SubtreeRetention| {
        let mut rt = Runtime::new(make_init(), live_view, (800.0, 600.0), Env::new(true, 1.0));
        rt.set_subtree_retention(mode);
        rt.frame(0.0);

        rt.state.tick += 1;
        commit(&mut rt);

        // Update live values without rebuilding memo.
        rt.state.opacity.set(0.5);
        rt.state.label.set("00:02");
        rt.frame(1.0 / 60.0);

        // Remove the sibling above `kept` so it enters exiting state, then settles and prunes.
        rt.state.show_temp = false;
        commit(&mut rt);
        for _ in 0..90 {
            rt.frame(1.0 / 60.0);
        }

        // Subsequent commit after `prune_exited` remapped indices.
        rt.state.tick += 1;
        commit(&mut rt);
        snapshot(&rt)
    };
    let (els_full, inst_full) = run(SubtreeRetention::FullRebuild);
    let (els_memo, inst_memo) = run(SubtreeRetention::BuilderMemo);
    let (els_ret, inst_ret) = run(SubtreeRetention::RetainedMounted);
    assert_eq!(els_full, els_memo);
    assert_eq!(els_memo, els_ret);
    assert_eq!(inst_full, inst_memo);
    assert_eq!(inst_memo, inst_ret);
}

#[test]
fn retention_tables_shrink_after_large_to_small_transition() {
    #[derive(Clone)]
    struct ShrinkState {
        count: usize,
        tick: u32,
    }

    fn shrink_view(s: &ShrinkState, _cx: &mut Cx<ShrinkState>) -> El {
        col()
            .key("root")
            .child(text(format!("tick {}", s.tick)).key("tick"))
            .children((0..s.count).map(|i| {
                memo(format!("item-{i}"), 1u32, move || {
                    row()
                        .padding(2.0)
                        .child(text(format!("row {i}")).key("label"))
                })
            }))
    }

    let mut rt = Runtime::new(ShrinkState { count: 300, tick: 0 }, shrink_view, (800.0, 2000.0), Env::new(true, 1.0));
    rt.frame(0.0);
    assert_eq!(rt.retained_subtree_stats().retained_subtrees, 300);

    rt.state.count = 2;
    rt.state.tick += 1;
    commit(&mut rt);
    for _ in 0..90 {
        rt.frame(1.0 / 60.0);
    }
    rt.state.tick += 1;
    commit(&mut rt);

    assert_eq!(rt.retained_subtree_stats().retained_subtrees, 2);
    assert!(rt.retained_subtree_stats().table_capacity <= 128);
}

#[test]
fn full_rebuild_mode_reruns_memo_closures_every_commit() {
    thread_local! {
        static CALLS: Cell<usize> = const { Cell::new(0) };
    }
    CALLS.set(0);

    fn counted_view(tick: &u32, _cx: &mut Cx<u32>) -> El {
        col()
            .key("root")
            .child(text(format!("{tick}")).key("tick"))
            .child(memo("sub", 1u32, || {
                CALLS.set(CALLS.get() + 1);
                el().key("inner").size(20.0, 20.0)
            }))
    }

    let mut rt = Runtime::new(0u32, counted_view, (200.0, 200.0), Env::new(true, 1.0));
    rt.set_subtree_retention(SubtreeRetention::FullRebuild);
    rt.frame(0.0);
    let after_init = CALLS.get();
    rt.state += 1;
    commit(&mut rt);
    assert!(CALLS.get() > after_init, "FullRebuild must re-invoke memo closure");
    assert_eq!(rt.retained_subtree_stats().spliced_subtrees, 0);
}
