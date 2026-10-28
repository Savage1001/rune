//! The layer-shell shell (`App::layer`): the same runtime and renderer as `window.rs`, presented
//! on a `zwlr_layer_surface_v1` instead of a winit window, for overlays, HUDs and notifications.
//!
//! - The surface is transparent: the runtime clears to the root's fill with its own alpha and
//!   the swapchain composites premultiplied.
//! - With `click_through`, the input region follows the runtime's hit targets after every frame,
//!   so the pointer passes through everywhere else.
//! - Frames are paced by `wl_surface.frame` callbacks while something animates; idle surfaces
//!   draw nothing. Fractional scale (`wp_fractional_scale_v1` + `wp_viewporter`) when the
//!   compositor has it, else the integer buffer scale.
//! - Opening maps a new layer surface; closing destroys it (layer surfaces can't hide). The
//!   runtime, GPU device and renderer stay. Unlike a window, a remapped overlay doesn't open
//!   settled: overlays come back because something new arrived, and that should animate in.
//! - Pointer input and cursor shapes (`wp_cursor_shape_v1`); no keyboard yet.

use std::ptr::NonNull;
use std::sync::Arc;
use std::time::Instant;

use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::ping::make_ping;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::cursor_shape::CursorShapeManager;
use smithay_client_toolkit::seat::pointer::{BTN_LEFT, PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer as WLayer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry, delegate_seat,
    registry_handlers,
};
use wayland_client::globals::{GlobalList, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{self, WpFractionalScaleV1};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;

use crate::app::{App, Layer, LayerLevel, Shared, WindowRequest};
use crate::el::{Cursor, Shell, Tx};
use crate::render::{Gpu, Renderer};
use crate::runtime::{Runtime, Waker};
use crate::tooling::{AppSpec, Args, runtime_for};

/// Whether this session can show layer surfaces: a Wayland display with `zwlr_layer_shell_v1`.
pub(crate) fn available() -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let (globals, _) = registry_queue_init::<Probe>(&conn).map_err(|e| e.to_string())?;
    let found = globals.contents().with_list(|l| l.iter().any(|g| g.interface == "zwlr_layer_shell_v1"));
    if found { Ok(()) } else { Err("the compositor has no zwlr_layer_shell_v1".into()) }
}

struct Probe;

impl Dispatch<wayland_client::protocol::wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for Probe {
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_registry::WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// The mapped surface. Fields drop in order: the wgpu surface before the Wayland surface.
struct Surf {
    target: Option<(wgpu::Surface<'static>, wgpu::SurfaceConfiguration)>,
    fractional: Option<WpFractionalScaleV1>,
    viewport: Option<WpViewport>,
    layer: LayerSurface,
    /// Logical size from the last configure.
    size: (u32, u32),
    /// Device pixels per point, in 120ths (the fractional-scale protocol's unit).
    scale120: u32,
    configured: bool,
    /// A frame callback is outstanding: draw when it fires, not before.
    frame_pending: bool,
    /// The input region last sent, in whole points.
    input: Option<Vec<[i32; 4]>>,
}

struct Host<S: 'static> {
    spec: AppSpec<S>,
    args: Args,
    layer_spec: Layer,
    keep_running: bool,
    on_close: Option<fn(&mut S, &mut Tx)>,
    release_gpu: bool,
    shared: Arc<Shared<S>>,
    rt: Runtime<S>,

    conn: Connection,
    qh: QueueHandle<Self>,
    registry: RegistryState,
    outputs: OutputState,
    seats: SeatState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    fractional: Option<WpFractionalScaleManagerV1>,
    viewporter: Option<WpViewporter>,
    cursor_shapes: Option<CursorShapeManager>,
    pointer: Option<(wl_pointer::WlPointer, Option<WpCursorShapeDeviceV1>)>,
    /// The last pointer enter serial and the shape set with it.
    enter_serial: u32,
    cursor: Option<Cursor>,

    surf: Option<Surf>,
    gpu: Option<(Gpu, Renderer, wgpu::TextureFormat)>,
    /// The next draw should happen (input, state change, animation).
    dirty: bool,
    idle: bool,
    last: Instant,
    exit: bool,
    clipboard: Option<arboard::Clipboard>,
}

pub(crate) fn run<S: 'static>(app: App<S>, args: Args) {
    let conn = Connection::connect_to_env().expect("Wayland display");
    let (globals, queue) = registry_queue_init::<Host<S>>(&conn).expect("Wayland registry");
    let qh = queue.handle();
    let mut event_loop: EventLoop<Host<S>> = EventLoop::try_new().expect("event loop");
    WaylandSource::new(conn.clone(), queue).insert(event_loop.handle()).expect("Wayland source");
    let (ping, ping_source) = make_ping().expect("wake-up source");
    event_loop
        .handle()
        .insert_source(ping_source, |_, _, host: &mut Host<S>| host.drain())
        .expect("wake-up source");
    let waker: Waker = Arc::new(move || ping.ping());

    let layer_spec = app.layer.clone().expect("App::layer");
    let (open, keep_running, on_close, release_gpu) = (app.open, app.keep_running, app.on_close, app.release_gpu);
    let (spec, state, shared) = app.into_parts(args.scene.as_deref());
    let size = args.size.unwrap_or(spec.size);
    // Layer surfaces report no appearance; `--theme light` picks light.
    let env = crate::runtime::Env { system_dark: args.theme.as_deref() != Some("light"), scale: 1.0 };
    let mut rt = runtime_for(&spec, state, size, env, waker.clone());
    rt.wall_start = Some(Instant::now());
    rt.transparent = true;

    let mut host = Host {
        spec,
        args,
        layer_spec,
        keep_running,
        on_close,
        release_gpu,
        shared: shared.clone(),
        rt,
        conn,
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        seats: SeatState::new(&globals, &qh),
        compositor: CompositorState::bind(&globals, &qh).expect("wl_compositor"),
        layer_shell: LayerShell::bind(&globals, &qh).expect("zwlr_layer_shell_v1"),
        fractional: bind(&globals, &qh, 1),
        viewporter: bind(&globals, &qh, 1),
        cursor_shapes: CursorShapeManager::bind(&globals, &qh).ok(),
        qh,
        pointer: None,
        enter_serial: 0,
        cursor: None,
        surf: None,
        gpu: None,
        dirty: false,
        idle: true,
        last: Instant::now(),
        exit: false,
        clipboard: arboard::Clipboard::new().ok(),
    };
    if open {
        host.open();
    }
    shared.connect(waker);
    while !host.exit {
        event_loop.dispatch(None, &mut host).expect("event loop");
        host.flush();
    }
}

fn bind<I, S: 'static>(globals: &GlobalList, qh: &QueueHandle<Host<S>>, version: u32) -> Option<I>
where
    I: Proxy + 'static,
    Host<S>: Dispatch<I, ()>,
{
    globals.bind(qh, version..=version, ()).ok()
}

fn shape(c: Cursor) -> Shape {
    match c {
        Cursor::Default => Shape::Default,
        Cursor::Pointer => Shape::Pointer,
        Cursor::Text => Shape::Text,
        Cursor::Grab => Shape::Grab,
        Cursor::Grabbing => Shape::Grabbing,
        Cursor::NotAllowed => Shape::NotAllowed,
        Cursor::Move => Shape::Move,
        Cursor::ResizeColumn => Shape::ColResize,
        Cursor::ResizeRow => Shape::RowResize,
    }
}

impl<S: 'static> Host<S> {
    fn scale(&self) -> f32 {
        self.surf.as_ref().map_or(1.0, |s| s.scale120 as f32 / 120.0)
    }

    /// Applies everything other threads asked for since the last wake-up.
    fn drain(&mut self) {
        let requests = self.shared.take();
        if !requests.updates.is_empty() {
            for f in requests.updates {
                self.rt.update(f);
            }
            self.dirty = true;
        }
        if requests.quit {
            self.exit = true;
            return;
        }
        if let Some(l) = requests.layer
            && l != self.layer_spec
        {
            self.layer_spec = l;
            if self.surf.is_some() {
                self.unmap();
                self.open();
            }
        }
        match requests.window {
            Some(WindowRequest::Open(_)) => self.open(),
            Some(WindowRequest::Close) => self.close(),
            None => {}
        }
        self.shell();
    }

    /// Carries out what a handler or update asked with `Tx::close_window` / `Tx::quit`.
    fn shell(&mut self) {
        match self.rt.shell_request.take() {
            Some(Shell::Quit) => self.exit = true,
            Some(Shell::Close) => self.close(),
            None => {}
        }
    }

    fn open(&mut self) {
        if self.surf.is_some() {
            return;
        }
        let l = &self.layer_spec;
        let surface = self.compositor.create_surface(&self.qh);
        let level = match l.level {
            LayerLevel::Background => WLayer::Background,
            LayerLevel::Bottom => WLayer::Bottom,
            LayerLevel::Top => WLayer::Top,
            LayerLevel::Overlay => WLayer::Overlay,
        };
        let layer = self.layer_shell.create_layer_surface(&self.qh, surface, level, Some(l.namespace), None);
        let mut anchor = Anchor::empty();
        anchor.set(Anchor::TOP, l.anchor.top);
        anchor.set(Anchor::RIGHT, l.anchor.right);
        anchor.set(Anchor::BOTTOM, l.anchor.bottom);
        anchor.set(Anchor::LEFT, l.anchor.left);
        layer.set_anchor(anchor);
        let size = self.args.size.or((l.size != (0.0, 0.0)).then_some(l.size)).unwrap_or(self.spec.size);
        let w = if l.anchor.left && l.anchor.right && l.size.0 == 0.0 { 0 } else { size.0.round() as u32 };
        let h = if l.anchor.top && l.anchor.bottom && l.size.1 == 0.0 { 0 } else { size.1.round() as u32 };
        layer.set_size(w, h);
        let m = l.margin.map(|v| v.round() as i32);
        layer.set_margin(m[0], m[1], m[2], m[3]);
        layer.set_exclusive_zone(l.exclusive_zone);
        layer.set_keyboard_interactivity(if l.keyboard { KeyboardInteractivity::OnDemand } else { KeyboardInteractivity::None });
        let wl = layer.wl_surface();
        let fractional = self.fractional.as_ref().map(|f| f.get_fractional_scale(wl, &self.qh, ()));
        let viewport = self.viewporter.as_ref().map(|v| v.get_viewport(wl, &self.qh, ()));
        if l.click_through {
            // Nothing takes input until the first frame says where the targets are.
            if let Ok(r) = Region::new(&self.compositor) {
                wl.set_input_region(Some(r.wl_region()));
            }
        }
        layer.commit();
        self.surf = Some(Surf {
            target: None,
            fractional,
            viewport,
            layer,
            size: (w, h),
            scale120: 120,
            configured: false,
            frame_pending: false,
            input: l.click_through.then(Vec::new),
        });
    }

    fn close(&mut self) {
        if self.surf.is_none() {
            return;
        }
        if let Some(f) = self.on_close {
            self.rt.update(f);
        }
        if !self.keep_running || self.rt.shell_request.take() == Some(Shell::Quit) {
            self.exit = true;
            return;
        }
        self.unmap();
        if self.release_gpu {
            self.gpu = None;
            self.rt.text.atlas.invalidate();
        }
        let _ = self.conn.flush();
    }

    fn unmap(&mut self) {
        self.rt.pointer_leave();
        self.cursor = None;
        if let Some(s) = self.surf.take() {
            if let Some(f) = &s.fractional {
                f.destroy();
            }
            if let Some(v) = &s.viewport {
                v.destroy();
            }
            drop(s);
        }
    }

    /// Fits the swapchain and runtime to the surface's size and scale.
    fn reconfigure(&mut self) {
        let scale = self.scale();
        let Some(s) = self.surf.as_mut() else { return };
        if !s.configured {
            return;
        }
        let (w, h) = s.size;
        let (pw, ph) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
        match &s.viewport {
            Some(v) => v.set_destination(w as i32, h as i32),
            None => s.layer.wl_surface().set_buffer_scale((s.scale120 / 120).max(1) as i32),
        }
        if s.target.is_none() {
            let display = NonNull::new(self.conn.backend().display_ptr().cast()).expect("display");
            let surface = NonNull::new(s.layer.wl_surface().id().as_ptr().cast()).expect("surface");
            let raw = wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display))),
                raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(surface)),
            };
            let instance = match &self.gpu {
                Some((gpu, ..)) => gpu.instance.clone(),
                None => Gpu::instance(),
            };
            // SAFETY: the display outlives the host, and `Surf` drops this surface before its
            // layer surface.
            let surface = unsafe { instance.create_surface_unsafe(raw) }.expect("wgpu surface");
            if self.gpu.is_none() {
                let gpu = Gpu::new(instance, Some(&surface));
                let format = wgpu::TextureFormat::Bgra8UnormSrgb;
                let renderer = Renderer::new(&gpu.device, format);
                self.gpu = Some((gpu, renderer, format));
            }
            let (gpu, renderer, format) = self.gpu.as_mut().unwrap();
            let caps = surface.get_capabilities(&gpu.adapter);
            let mut config = surface.get_default_config(&gpu.adapter, pw, ph).expect("surface config");
            config.format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(config.format);
            config.alpha_mode = [wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::Inherit]
                .into_iter()
                .find(|m| caps.alpha_modes.contains(m))
                .unwrap_or(config.alpha_mode);
            // Frame callbacks pace the frames; mailbox never blocks an occluded surface.
            config.present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
                wgpu::PresentMode::Mailbox
            } else {
                wgpu::PresentMode::AutoVsync
            };
            config.desired_maximum_frame_latency = 2;
            if *format != config.format {
                *renderer = Renderer::new(&gpu.device, config.format);
                *format = config.format;
                self.rt.text.atlas.invalidate();
            }
            s.target = Some((surface, config));
        }
        let (gpu, ..) = self.gpu.as_ref().unwrap();
        let (surface, config) = s.target.as_mut().unwrap();
        config.width = pw;
        config.height = ph;
        surface.configure(&gpu.device, config);
        self.rt.set_viewport((w as f32, h as f32), scale);
        self.dirty = true;
    }

    /// Draws if something asked to and the compositor isn't still showing the last frame.
    fn flush(&mut self) {
        if self.dirty && self.surf.as_ref().is_some_and(|s| s.configured && !s.frame_pending) {
            self.redraw();
            self.shell();
        }
        let _ = self.conn.flush();
    }

    fn redraw(&mut self) {
        self.dirty = false;
        let (Some(s), Some((gpu, renderer, _))) = (self.surf.as_mut(), self.gpu.as_mut()) else { return };
        let Some((surface, config)) = s.target.as_mut() else { return };
        let now = Instant::now();
        // After an idle stretch, start new motion from its first pose.
        let dt = if self.idle { 1.0 / 120.0 } else { (now - self.last).as_secs_f64().min(1.0 / 30.0) };
        self.last = now;
        let rt = &mut self.rt;
        let animating = rt.frame(dt);
        if let Some(c) = rt.copy_request.take()
            && let Some(cb) = self.clipboard.as_mut()
        {
            let _ = cb.set_text(c);
        }
        let cursor = rt.cursor();
        if self.cursor != Some(cursor)
            && let Some((_, Some(device))) = &self.pointer
        {
            device.set_shape(self.enter_serial, shape(cursor));
            self.cursor = Some(cursor);
        }
        let wl = s.layer.wl_surface().clone();
        if let Some(sent) = s.input.as_mut() {
            let mut rects: Vec<[i32; 4]> = rt
                .hit_rects()
                .filter(|r| r[2] > r[0] && r[3] > r[1])
                .map(|r| {
                    let (x0, y0) = (r[0].floor() as i32, r[1].floor() as i32);
                    [x0, y0, r[2].ceil() as i32 - x0, r[3].ceil() as i32 - y0]
                })
                .collect();
            rects.sort_unstable();
            rects.dedup();
            if *sent != rects
                && let Ok(region) = Region::new(&self.compositor)
            {
                for r in &rects {
                    region.add(r[0], r[1], r[2], r[3]);
                }
                wl.set_input_region(Some(region.wl_region()));
                *sent = rects;
            }
        }
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                surface.configure(&gpu.device, config);
                self.dirty = true;
                return;
            }
            _ => {
                self.dirty = true;
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        renderer.prepare(&gpu.device, &gpu.queue, &mut rt.text.atlas, &rt.instances, (config.width, config.height));
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        renderer.draw(&mut enc, &view, rt.clear_color());
        gpu.queue.submit([enc.finish()]);
        self.idle = !(animating || rt.has_pending_input());
        if !self.idle {
            // Committed with this frame's present; the next draw waits for it.
            wl.frame(&self.qh, wl.clone());
            s.frame_pending = true;
        }
        gpu.queue.present(frame);
    }

    fn pointer_event(&mut self, e: &PointerEvent) {
        let (x, y) = (e.position.0 as f32, e.position.1 as f32);
        match e.kind {
            PointerEventKind::Enter { serial } => {
                self.enter_serial = serial;
                self.cursor = None;
                self.rt.pointer_move(x, y);
            }
            PointerEventKind::Leave { .. } => self.rt.pointer_leave(),
            PointerEventKind::Motion { .. } => self.rt.pointer_move(x, y),
            PointerEventKind::Press { button: BTN_LEFT, .. } => self.rt.pointer_down(x, y),
            PointerEventKind::Release { button: BTN_LEFT, .. } => self.rt.pointer_up(x, y),
            PointerEventKind::Axis { vertical, .. } => {
                // Wayland scrolls positive downward; Rune's wheel is positive upward.
                let dy = if vertical.discrete != 0 { -vertical.discrete as f32 * 40.0 } else { -vertical.absolute as f32 };
                if dy != 0.0 {
                    self.rt.wheel(x, y, dy);
                }
            }
            _ => return,
        }
        self.dirty = true;
    }

    fn is_ours(&self, surface: &wl_surface::WlSurface) -> bool {
        self.surf.as_ref().is_some_and(|s| s.layer.wl_surface() == surface)
    }
}

impl<S: 'static> CompositorHandler for Host<S> {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        if !self.is_ours(surface) {
            return;
        }
        let s = self.surf.as_mut().unwrap();
        // The fractional scale, when present, is the one that counts.
        if s.fractional.is_none() && s.scale120 != factor.max(1) as u32 * 120 {
            s.scale120 = factor.max(1) as u32 * 120;
            self.reconfigure();
        }
    }

    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        if let Some(s) = self.surf.as_mut().filter(|s| s.layer.wl_surface() == surface) {
            s.frame_pending = false;
            self.dirty = true;
        }
    }

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl<S: 'static> LayerShellHandler for Host<S> {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        // The compositor took the surface away (its output went): drop it like a close.
        if self.surf.as_ref().is_some_and(|s| &s.layer == layer) {
            self.close();
        }
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, c: LayerSurfaceConfigure, _: u32) {
        let Some(s) = self.surf.as_mut().filter(|s| &s.layer == layer) else { return };
        let (w, h) = (
            if c.new_size.0 == 0 { s.size.0.max(1) } else { c.new_size.0 },
            if c.new_size.1 == 0 { s.size.1.max(1) } else { c.new_size.1 },
        );
        let first = !s.configured;
        s.configured = true;
        if first || (w, h) != s.size {
            s.size = (w, h);
            self.idle = true;
            self.reconfigure();
        }
    }
}

impl<S: 'static> Dispatch<WpFractionalScaleV1, ()> for Host<S> {
    fn event(host: &mut Self, proxy: &WpFractionalScaleV1, event: wp_fractional_scale_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let wp_fractional_scale_v1::Event::PreferredScale { scale } = event else { return };
        let Some(s) = host.surf.as_mut().filter(|s| s.fractional.as_ref() == Some(proxy)) else { return };
        if s.scale120 != scale {
            s.scale120 = scale;
            host.reconfigure();
        }
    }
}

impl<S: 'static> Dispatch<WpFractionalScaleManagerV1, ()> for Host<S> {
    fn event(_: &mut Self, _: &WpFractionalScaleManagerV1, _: <WpFractionalScaleManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl<S: 'static> Dispatch<WpViewporter, ()> for Host<S> {
    fn event(_: &mut Self, _: &WpViewporter, _: <WpViewporter as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl<S: 'static> Dispatch<WpViewport, ()> for Host<S> {
    fn event(_: &mut Self, _: &WpViewport, _: <WpViewport as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl<S: 'static> OutputHandler for Host<S> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl<S: 'static> SeatHandler for Host<S> {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            let Ok(pointer) = self.seats.get_pointer(qh, &seat) else { return };
            let device = self.cursor_shapes.as_ref().map(|m| m.get_shape_device(&pointer, qh));
            self.pointer = Some((pointer, device));
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer
            && let Some((pointer, device)) = self.pointer.take()
        {
            if let Some(d) = device {
                d.destroy();
            }
            pointer.release();
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl<S: 'static> PointerHandler for Host<S> {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for e in events {
            if self.is_ours(&e.surface) {
                self.pointer_event(e);
            }
        }
    }
}

impl<S: 'static> ProvidesRegistryState for Host<S> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(@<S: 'static> Host<S>);
delegate_output!(@<S: 'static> Host<S>);
delegate_seat!(@<S: 'static> Host<S>);
delegate_pointer!(@<S: 'static> Host<S>);
delegate_layer!(@<S: 'static> Host<S>);
delegate_registry!(@<S: 'static> Host<S>);
