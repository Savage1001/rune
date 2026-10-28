//! CPU-only representation inventory; no window, renderer or native event loop.
use rune::{Cx, El, Env, Runtime, col, el, row, text};

fn view(count: &usize, _: &mut Cx<usize>) -> El {
    col().w_full().children((0..*count).map(|i| {
        row().key(format!("row-{i}")).w_full().h(32.0)
            .child(text(format!("Row {i}")).key("text"))
            .child(el().key("button").size(20.0, 20.0).on_press(|_: &mut usize| {}))
    }))
}

fn main() {
    assert!(!cfg!(debug_assertions), "use --profile perf");
    let mut rt = Runtime::new(500usize, view, (1000.0, 900.0), Env::new(true, 2.0));
    for count in [500, 500, 10, 10, 500] {
        rt.state = count;
        rt.invalidate();
        rt.frame(0.0);
        println!("METRIC probe=storage-swap rows={count}");
        rt.print_storage_inventory();
    }
}
