//! Flowchart node geometry, outlines, connectors and layered scene drawing.

use rune::{Align, TextStyle};

use super::flowchart::{Dir, Edge, Flowchart, Head, Line, NodeShape};
use super::layered::{GCluster, GEdge, Graph, Spacing, layout};
use super::{
    Canvas, DiagramStyle, Measured, Scene, arc, arrowhead, dist, ellipse, fit, measure, room,
    rounded_rect, smooth, unit,
};

const PAD: [f32; 2] = [30.0, 22.0];
const SPACING: Spacing = Spacing { node: 36.0, rank: 44.0, cluster: 14.0 };

struct Sized {
    label: Measured,
    size: [f32; 2],
}

pub(crate) fn node_size(shape: NodeShape, t: [f32; 2]) -> [f32; 2] {
    let [w, h] = [t[0] + PAD[0], (t[1] + PAD[1]).max(40.0)];
    match shape {
        NodeShape::Rect | NodeShape::Round => [w.max(60.0), h],
        NodeShape::Stadium | NodeShape::Hexagon => [w + h / 2.0, h],
        NodeShape::Subroutine => [w + 16.0, h],
        NodeShape::Asymmetric => [w + h / 3.0, h],
        NodeShape::Cylinder => [w.max(60.0), h + 16.0],
        NodeShape::Parallelogram | NodeShape::Trapezoid => [w + h * 0.6, h],
        NodeShape::Circle => {
            let d = (t[0] * t[0] + t[1] * t[1]).sqrt() + 20.0;
            [d.max(48.0), d.max(48.0)]
        }
        NodeShape::Diamond => {
            let a = t[0] / 2.0 + t[1] + 12.0;
            let b = t[1] / 2.0 * a / (t[1] + 12.0) + 10.0;
            [2.0 * a, (2.0 * b).max(40.0)]
        }
    }
}

/// Where the line from `c` toward `toward` leaves a node of `shape` and `size` centred at `c`.
pub(crate) fn boundary(shape: NodeShape, c: [f32; 2], size: [f32; 2], toward: [f32; 2]) -> [f32; 2] {
    let (dx, dy) = (toward[0] - c[0], toward[1] - c[1]);
    if dx.abs() < 1e-3 && dy.abs() < 1e-3 {
        return c;
    }
    let (a, b) = (size[0] / 2.0, size[1] / 2.0);
    let t = match shape {
        NodeShape::Circle => 1.0 / ((dx / a).powi(2) + (dy / b).powi(2)).sqrt(),
        NodeShape::Diamond => 1.0 / (dx.abs() / a + dy.abs() / b),
        _ => {
            let tx = if dx.abs() > 1e-3 { a / dx.abs() } else { f32::MAX };
            let ty = if dy.abs() > 1e-3 { b / dy.abs() } else { f32::MAX };
            tx.min(ty)
        }
    };
    let t = t.min(1.0);
    [c[0] + dx * t, c[1] + dy * t]
}

pub(crate) fn outline(shape: NodeShape, c: [f32; 2], s: [f32; 2]) -> Vec<[f32; 2]> {
    let (x0, y0, x1, y1) = (c[0] - s[0] / 2.0, c[1] - s[1] / 2.0, c[0] + s[0] / 2.0, c[1] + s[1] / 2.0);
    let h = s[1];
    match shape {
        NodeShape::Rect | NodeShape::Subroutine => rounded_rect(x0, y0, x1, y1, 3.0),
        NodeShape::Round => rounded_rect(x0, y0, x1, y1, 10.0),
        NodeShape::Stadium => rounded_rect(x0, y0, x1, y1, h / 2.0),
        NodeShape::Circle => ellipse(c, s[0] / 2.0, s[1] / 2.0),
        NodeShape::Diamond => vec![[c[0], y0], [x1, c[1]], [c[0], y1], [x0, c[1]]],
        NodeShape::Hexagon => {
            let i = h / 4.0;
            vec![[x0 + i, y0], [x1 - i, y0], [x1, c[1]], [x1 - i, y1], [x0 + i, y1], [x0, c[1]]]
        }
        NodeShape::Asymmetric => vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0 + h / 3.0, c[1]]],
        NodeShape::Parallelogram => {
            let k = h * 0.3;
            vec![[x0 + k, y0], [x1, y0], [x1 - k, y1], [x0, y1]]
        }
        NodeShape::Trapezoid => {
            let k = h * 0.3;
            vec![[x0 + k, y0], [x1 - k, y0], [x1, y1], [x0, y1]]
        }
        NodeShape::Cylinder => {
            let ry = 8.0;
            let rx = s[0] / 2.0;
            let mut pts = arc([c[0], y0 + ry], rx, ry, 180.0, 360.0, 20);
            pts.extend(arc([c[0], y1 - ry], rx, ry, 0.0, 180.0, 20));
            pts
        }
    }
}

pub(crate) fn draw(f: &Flowchart, paint: &DiagramStyle) -> Option<Scene> {
    let nodes: Vec<Sized> = f
        .nodes
        .iter()
        .map(|n| {
            let label = measure(&n.label, paint.font);
            Sized { size: node_size(n.shape, label.size), label }
        })
        .collect();
    let edge_font = TextStyle { size: paint.font.size - 1.0, line_height: paint.font.line_height - 2.0, ..paint.font };
    let labels: Vec<Option<Measured>> = f.edges.iter().map(|e| e.label.as_ref().map(|l| measure(l, edge_font))).collect();
    let title_font = TextStyle { weight: 600.0, ..edge_font };
    let titles: Vec<Measured> = f.subgraphs.iter().map(|s| measure(&s.title, title_font)).collect();
    let fonts = Fonts { node: paint.font, edge: edge_font, title: title_font };
    let mut canvas = draw_in(f, f.dir, &nodes, &labels, &titles, fonts, paint)?;
    if canvas.width() > room(paint) && matches!(f.dir, Dir::Right | Dir::Left) {
        let down = draw_in(f, if f.dir == Dir::Right { Dir::Down } else { Dir::Up }, &nodes, &labels, &titles, fonts, paint)?;
        if down.width() < canvas.width() {
            canvas = down;
        }
    }
    let scale = fit(canvas.width(), paint);
    canvas.finish(scale)
}

#[derive(Clone, Copy)]
struct Fonts {
    node: TextStyle,
    edge: TextStyle,
    title: TextStyle,
}

fn draw_in(f: &Flowchart, dir: Dir, nodes: &[Sized], labels: &[Option<Measured>], titles: &[Measured], fonts: Fonts, paint: &DiagramStyle) -> Option<Canvas> {
    let across = matches!(dir, Dir::Right | Dir::Left);
    let ax = |s: [f32; 2]| if across { [s[1], s[0]] } else { s };
    let flip = matches!(dir, Dir::Up | Dir::Left);
    let label_box = |m: &Measured| [m.size[0] + 10.0, m.size[1] + 4.0];
    let graph = Graph {
        sizes: nodes.iter().map(|n| ax(n.size)).collect(),
        edges: f
            .edges
            .iter()
            .zip(labels)
            .map(|(e, l)| {
                let (from, to) = if flip { (e.to, e.from) } else { (e.from, e.to) };
                GEdge { from, to, label: l.as_ref().map(|m| ax(label_box(m))), minlen: e.minlen }
            })
            .collect(),
        node_cluster: f.nodes.iter().map(|n| n.cluster).collect(),
        clusters: f
            .subgraphs
            .iter()
            .zip(titles)
            .map(|(s, t)| {
                let lead = if t.text.is_empty() { 0.0 } else { t.size[1] + 8.0 };
                GCluster { parent: s.parent, lead: if across { [lead, 0.0] } else { [0.0, lead] } }
            })
            .collect(),
    };
    let placed = layout(&graph, &SPACING)?;
    let pt = |p: [f32; 2]| if across { [p[1], p[0]] } else { p };
    let mut c = Canvas::default();

    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = f.subgraphs[i].parent {
            i = p;
            d += 1;
            if d > f.subgraphs.len() {
                break;
            }
        }
        d
    };
    let mut order: Vec<usize> = (0..f.subgraphs.len()).collect();
    order.sort_by_key(|&i| depth(i));
    for i in order {
        let b = placed.clusters[i];
        if b == [0.0; 4] {
            continue;
        }
        let (p0, p1) = (pt([b[0], b[1]]), pt([b[2], b[3]]));
        c.shape(rounded_rect(p0[0], p0[1], p1[0], p1[1], 6.0), paint.cluster_fill, paint.surface_border, 1.0);
        c.label_at(&titles[i], [p0[0] + 10.0, p0[1] + 7.0], fonts.title, paint.muted, Align::Start);
    }

    let centers: Vec<[f32; 2]> = placed.centers.iter().map(|&p| pt(p)).collect();
    for (i, e) in f.edges.iter().enumerate() {
        let mut path: Vec<[f32; 2]> = placed.paths[i].iter().map(|&p| pt(p)).collect();
        if flip {
            path.reverse();
        }
        let (from, to) = (e.from, e.to);
        let mid = if path.len() <= 1 {
            let s = labels[i].as_ref().map_or(0.0, |m| label_box(m)[0]);
            self_loop(&mut c, e, centers[from], nodes[from].size, paint).map(|at| [at[0] + s / 2.0, at[1]])
        } else {
            let n = path.len();
            path[0] = boundary(f.nodes[from].shape, centers[from], nodes[from].size, path[1]);
            path[n - 1] = boundary(f.nodes[to].shape, centers[to], nodes[to].size, path[n - 2]);
            connector(&mut c, &mut path, e, paint);
            placed.labels[i].map(pt)
        };
        if let (Some(m), Some(at)) = (&labels[i], mid) {
            let s = label_box(m);
            c.fill(rounded_rect(at[0] - s[0] / 2.0, at[1] - s[1] / 2.0, at[0] + s[0] / 2.0, at[1] + s[1] / 2.0, 3.0), paint.surface);
            c.label(m, at, fonts.edge, paint.text, Align::Center);
        }
    }

    for (i, n) in f.nodes.iter().enumerate() {
        let (ctr, size) = (centers[i], nodes[i].size);
        let (fill, border, text_color) = n.style.resolve(paint);
        c.shape(outline(n.shape, ctr, size), fill, border, 1.0);
        let (x0, y0, x1, y1) = (ctr[0] - size[0] / 2.0, ctr[1] - size[1] / 2.0, ctr[0] + size[0] / 2.0, ctr[1] + size[1] / 2.0);
        match n.shape {
            NodeShape::Subroutine => {
                c.stroke(vec![[x0 + 8.0, y0], [x0 + 8.0, y1]], false, 1.0, border);
                c.stroke(vec![[x1 - 8.0, y0], [x1 - 8.0, y1]], false, 1.0, border);
            }
            NodeShape::Cylinder => c.stroke(arc([ctr[0], y0 + 8.0], size[0] / 2.0, 8.0, 0.0, 180.0, 20), false, 1.0, border),
            _ => {}
        }
        let center = if n.shape == NodeShape::Cylinder { [ctr[0], ctr[1] + 4.0] } else { ctr };
        c.label(&nodes[i].label, center, fonts.node, text_color, Align::Center);
    }
    Some(c)
}

const ARROW: f32 = 8.0;

/// Draws `e` along `path` (endpoints on the node outlines), heads included.
pub(crate) fn connector(c: &mut Canvas, path: &mut [[f32; 2]], e: &Edge, paint: &DiagramStyle) {
    let n = path.len();
    let (tip_end, tip_start) = (path[n - 1], path[0]);
    let (dir_end, dir_start) = (unit(path[n - 2], path[n - 1]), unit(path[1], path[0]));
    let inset = |h: Head| match h {
        Head::None => 0.0,
        Head::Arrow => ARROW - 1.0,
        Head::Circle => 8.0,
        Head::Cross => 4.0,
    };
    let shorten = |p: [f32; 2], d: [f32; 2], by: f32| [p[0] - d[0] * by, p[1] - d[1] * by];
    if dist(path[n - 2], path[n - 1]) > inset(e.end) + 1.0 {
        path[n - 1] = shorten(path[n - 1], dir_end, inset(e.end));
    }
    if dist(path[0], path[1]) > inset(e.start) + 1.0 {
        path[0] = shorten(path[0], dir_start, inset(e.start));
    }
    let width = match e.line {
        Line::Thick => 2.5,
        _ => 1.25,
    };
    let curve = smooth(path);
    match e.line {
        Line::Invisible => return,
        Line::Dotted => c.dashed(&curve, width, paint.line, 3.0, 3.0),
        _ => c.stroke(curve, false, width, paint.line),
    }
    head(c, e.end, tip_end, dir_end, width, paint);
    head(c, e.start, tip_start, dir_start, width, paint);
}

fn head(c: &mut Canvas, h: Head, tip: [f32; 2], d: [f32; 2], width: f32, paint: &DiagramStyle) {
    let back = |by: f32| [tip[0] - d[0] * by, tip[1] - d[1] * by];
    match h {
        Head::None => {}
        Head::Arrow => c.fill(arrowhead(tip, d, ARROW + width, 7.0 + width), paint.line),
        Head::Circle => c.shape(ellipse(back(4.0), 4.0, 4.0), paint.node_fill, paint.line, width),
        Head::Cross => {
            let m = back(4.0);
            let (a, b) = ([d[0] + d[1], d[1] - d[0]], [d[0] - d[1], d[1] + d[0]]);
            let k = 3.5 / std::f32::consts::SQRT_2;
            c.stroke(vec![[m[0] - a[0] * k, m[1] - a[1] * k], [m[0] + a[0] * k, m[1] + a[1] * k]], false, width, paint.line);
            c.stroke(vec![[m[0] - b[0] * k, m[1] - b[1] * k], [m[0] + b[0] * k, m[1] + b[1] * k]], false, width, paint.line);
        }
    }
}

/// A loop off the node's right side; returns the left middle of where its label goes.
pub(crate) fn self_loop(c: &mut Canvas, e: &Edge, ctr: [f32; 2], size: [f32; 2], paint: &DiagramStyle) -> Option<[f32; 2]> {
    let x = ctr[0] + size[0] / 2.0;
    let (a, b) = ([x, ctr[1] - size[1] / 4.0], [x, ctr[1] + size[1] / 4.0]);
    let mut path = [a, [x + 18.0, a[1] - 4.0], [x + 26.0, ctr[1]], [x + 18.0, b[1] + 4.0], b];
    connector(c, &mut path, e, paint);
    Some([x + 30.0, ctr[1]])
}
