use std::{num::NonZeroU32, time::Duration};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::client::{
        Connection, Dispatch, QueueHandle,
        globals::registry_queue_init,
        protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface},
    },
    reexports::{
        calloop::{EventLoop, LoopHandle},
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    viewporter::client::{wp_viewport, wp_viewporter},
};

use crate::{
    commands::{
        CommandEntry, MAX_VISIBLE_RESULTS, SearchResult, launch_command, launch_path,
        load_commands, search_results,
    },
    render::{
        FontRenderer, Rect, clear, fill_rect, fill_rect_clipped_to_rounded, fill_rounded_rect,
        head_text_to_width, load_font, scale_px, tail_text_to_width,
    },
};

const LAUNCHER_WIDTH: u32 = 720;
const LAUNCHER_HEIGHT: u32 = 288;
const SURFACE_NAMESPACE: &str = "spark";
const INPUT_FONT_SIZE: f32 = 16.0;
const RESULT_FONT_SIZE: f32 = 16.0;
const TEXT_MARGIN_X: i32 = 16;
const TEXT_MARGIN_Y: i32 = 12;
const INPUT_HEIGHT: i32 = 36;
const SEPARATOR_MARGIN_TOP: i32 = 12;
const RESULT_MARGIN_TOP: i32 = 1;
const RESULT_TEXT_TOP_PADDING: i32 = 4;
const RESULT_EMPTY_TEXT_TOP_PADDING: i32 = 4;
const PANEL_RADIUS: i32 = 18;
const CARET_WIDTH: i32 = 2;

const COLOR_TRANSPARENT: u32 = 0x0000_0000;
const COLOR_BACKGROUND: u32 = 0xFF09_090B;
const COLOR_TEXT: u32 = 0xFFA1_A1AA;
const COLOR_PLACEHOLDER: u32 = 0xFF71_717A;
const COLOR_HIGHLIGHT: u32 = 0xFF79_697B;
const COLOR_SELECTION: u32 = 0xFF18_181B;
const COLOR_SEPARATOR: u32 = 0xFF18_181B;

pub(crate) fn run() {
    let font = load_font();
    let commands = load_commands();
    let conn = Connection::connect_to_env().expect("failed to connect to the Wayland compositor");
    let (globals, event_queue) =
        registry_queue_init(&conn).expect("failed to initialize the Wayland registry");
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<SparkLauncher> =
        EventLoop::try_new().expect("failed to initialize the event loop");
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .expect("failed to attach the Wayland source to the event loop");

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor is not available");
    let layer_shell = LayerShell::bind(&globals, &qh).expect("layer-shell is not available");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm is not available");
    let fractional_scale_manager = globals
        .bind::<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, ())
        .ok();
    let viewporter = globals
        .bind::<wp_viewporter::WpViewporter, _, _>(&qh, 1..=1, ())
        .ok();

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some(SURFACE_NAMESPACE),
        None,
    );
    layer.set_anchor(Anchor::empty());
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.set_size(LAUNCHER_WIDTH, LAUNCHER_HEIGHT);
    layer.commit();
    let viewport = viewporter
        .as_ref()
        .map(|viewporter| viewporter.get_viewport(layer.wl_surface(), &qh, ()));
    let fractional_scale = fractional_scale_manager
        .as_ref()
        .map(|manager| manager.get_fractional_scale(layer.wl_surface(), &qh, ()));

    let pool = SlotPool::new((LAUNCHER_WIDTH * LAUNCHER_HEIGHT * 4) as usize, &shm)
        .expect("failed to create the shared memory pool");

    let mut app = SparkLauncher {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        exit: false,
        first_configure: true,
        width: LAUNCHER_WIDTH,
        height: LAUNCHER_HEIGHT,
        pool,
        layer,
        viewport,
        _fractional_scale: fractional_scale,
        keyboard: None,
        keyboard_focus: false,
        modifiers: Modifiers::default(),
        loop_handle,
        font,
        buffer_scale: 1,
        preferred_fractional_scale: None,
        commands,
        visible_results: Vec::new(),
        selected_result: 0,
        query: String::new(),
    };
    app.refresh_results();

    while !app.exit {
        event_loop
            .dispatch(Duration::from_secs(1), &mut app)
            .expect("Wayland event dispatch failed");
    }
}

struct SparkLauncher {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    exit: bool,
    first_configure: bool,
    width: u32,
    height: u32,
    pool: SlotPool,
    layer: LayerSurface,
    viewport: Option<wp_viewport::WpViewport>,
    _fractional_scale: Option<wp_fractional_scale_v1::WpFractionalScaleV1>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    keyboard_focus: bool,
    modifiers: Modifiers,
    loop_handle: LoopHandle<'static, SparkLauncher>,
    font: FontRenderer,
    buffer_scale: i32,
    preferred_fractional_scale: Option<f64>,
    commands: Vec<CommandEntry>,
    visible_results: Vec<SearchResult>,
    selected_result: usize,
    query: String,
}

impl SparkLauncher {
    fn draw(&mut self) {
        self.configure_surface_scale();

        let scale = self.render_scale();
        let buffer_width = scale_px(self.width as i32, scale);
        let buffer_height = scale_px(self.height as i32, scale);
        let width = buffer_width as usize;
        let height = buffer_height as usize;
        let stride = buffer_width * 4;

        let (buffer, canvas) = self
            .pool
            .create_buffer(
                buffer_width,
                buffer_height,
                stride,
                wl_shm::Format::Argb8888,
            )
            .expect("failed to allocate a shared memory buffer");

        clear(canvas, COLOR_TRANSPARENT);

        let panel_rect = Rect::new(0, 0, buffer_width, buffer_height);
        let panel_radius = scale_px(PANEL_RADIUS, scale);
        fill_rounded_rect(
            canvas,
            width,
            height,
            panel_rect,
            panel_radius,
            COLOR_BACKGROUND,
        );

        let input_font_size = INPUT_FONT_SIZE * scale as f32;
        let result_font_size = RESULT_FONT_SIZE * scale as f32;
        let margin_x = scale_px(TEXT_MARGIN_X, scale);
        let margin_y = scale_px(TEXT_MARGIN_Y, scale);
        let input_height = scale_px(INPUT_HEIGHT, scale);
        let separator_margin_top = scale_px(SEPARATOR_MARGIN_TOP, scale);
        let result_margin_top = scale_px(RESULT_MARGIN_TOP, scale);
        let result_text_top_padding = scale_px(RESULT_TEXT_TOP_PADDING, scale);
        let empty_text_top_padding = scale_px(RESULT_EMPTY_TEXT_TOP_PADDING, scale);
        let caret_width = scale_px(CARET_WIDTH, scale).max(1);

        let input_metrics = self.font.line_metrics(input_font_size);
        let input_line_top = margin_y + ((input_height - input_metrics.height).max(0) / 2);
        let input_baseline_y = input_line_top + input_metrics.ascent;
        let available_width = buffer_width - margin_x * 2;
        let visible_query =
            tail_text_to_width(&self.font, input_font_size, &self.query, available_width);

        if visible_query.is_empty() {
            self.font.draw_text(
                canvas,
                width,
                height,
                margin_x,
                input_baseline_y,
                input_font_size,
                COLOR_PLACEHOLDER,
                "exec",
            );
        } else {
            self.font.draw_text(
                canvas,
                width,
                height,
                margin_x,
                input_baseline_y,
                input_font_size,
                COLOR_TEXT,
                &visible_query,
            );
        }

        if self.keyboard_focus {
            let caret_x = if self.query.is_empty() {
                margin_x
            } else {
                margin_x
                    + self
                        .font
                        .measure_text_width(input_font_size, &visible_query)
            };
            fill_rect(
                canvas,
                width,
                height,
                Rect::new(caret_x, input_line_top, caret_width, input_metrics.height),
                COLOR_HIGHLIGHT,
            );
        }

        let separator_y = margin_y + input_height + separator_margin_top;
        fill_rect(
            canvas,
            width,
            height,
            Rect::new(0, separator_y, buffer_width, 1),
            COLOR_SEPARATOR,
        );

        let result_metrics = self.font.line_metrics(result_font_size);
        let list_top = separator_y + result_margin_top;
        let list_width = buffer_width - margin_x * 2;
        let list_height = (buffer_height - list_top).max(0);
        let slot_count = MAX_VISIBLE_RESULTS as i32;
        if self.visible_results.is_empty() {
            self.font.draw_text(
                canvas,
                width,
                height,
                margin_x,
                list_top + empty_text_top_padding + result_metrics.ascent,
                result_font_size,
                COLOR_PLACEHOLDER,
                "No matches",
            );
        } else {
            for (row, result) in self.visible_results.iter().enumerate() {
                let entry = &self.commands[result.index];
                let row_index = row as i32;
                let row_top = list_top + (row_index * list_height) / slot_count;
                let row_bottom = list_top + ((row_index + 1) * list_height) / slot_count;
                let row_height = row_bottom - row_top;
                if row == self.selected_result {
                    fill_rect_clipped_to_rounded(
                        canvas,
                        width,
                        height,
                        Rect::new(0, row_top, buffer_width, row_height),
                        panel_rect,
                        panel_radius,
                        COLOR_SELECTION,
                    );
                }
                let baseline_y = row_top + result_text_top_padding + result_metrics.ascent;
                let visible_name =
                    head_text_to_width(&self.font, result_font_size, &entry.name, list_width);
                let highlight_positions = result
                    .matched_positions
                    .iter()
                    .copied()
                    .take_while(|position| *position < visible_name.chars().count())
                    .collect::<Vec<_>>();
                self.font.draw_highlighted_text(
                    canvas,
                    width,
                    height,
                    margin_x,
                    baseline_y,
                    result_font_size,
                    COLOR_TEXT,
                    COLOR_HIGHLIGHT,
                    &visible_name,
                    &highlight_positions,
                );
            }
        }

        self.layer
            .wl_surface()
            .damage_buffer(0, 0, buffer_width, buffer_height);
        buffer
            .attach_to(self.layer.wl_surface())
            .expect("failed to attach the shared memory buffer");
        self.layer.commit();
    }

    fn render_scale(&self) -> f64 {
        if self.viewport.is_some() {
            self.preferred_fractional_scale
                .unwrap_or(self.buffer_scale.max(1) as f64)
        } else {
            self.buffer_scale.max(1) as f64
        }
    }

    fn configure_surface_scale(&self) {
        if let Some(viewport) = self.viewport.as_ref() {
            self.layer.wl_surface().set_buffer_scale(1);
            viewport.set_destination(self.width as i32, self.height as i32);
        } else {
            self.layer
                .wl_surface()
                .set_buffer_scale(self.buffer_scale.max(1));
        }
    }

    fn handle_key_event(&mut self, event: KeyEvent, repeated: bool) {
        let redraw = match event.keysym {
            Keysym::Escape if !repeated => {
                self.exit = true;
                return;
            }
            Keysym::Return | Keysym::KP_Enter if !repeated => {
                self.launch();
                false
            }
            Keysym::Up | Keysym::KP_Up => self.move_selection(-1),
            Keysym::Down | Keysym::KP_Down => self.move_selection(1),
            Keysym::BackSpace => {
                let changed = self.query.pop().is_some();
                if changed {
                    self.refresh_results();
                }
                changed
            }
            _ => self.insert_text(&event),
        };

        if redraw && !self.exit {
            self.draw();
        }
    }

    fn insert_text(&mut self, event: &KeyEvent) -> bool {
        if self.modifiers.ctrl || self.modifiers.alt || self.modifiers.logo {
            return false;
        }

        let Some(text) = event.utf8.as_deref() else {
            return false;
        };

        let filtered: String = text
            .chars()
            .filter(|character| !character.is_control())
            .collect();
        if filtered.is_empty() {
            return false;
        }

        self.query.push_str(&filtered);
        self.refresh_results();
        true
    }

    fn refresh_results(&mut self) {
        self.visible_results = search_results(&self.commands, &self.query);
        self.selected_result = 0;
    }

    fn move_selection(&mut self, delta: i32) -> bool {
        if self.visible_results.is_empty() {
            self.selected_result = 0;
            return false;
        }

        let last_index = self.visible_results.len() as i32 - 1;
        let next_index = (self.selected_result as i32 + delta).clamp(0, last_index);
        let changed = next_index as usize != self.selected_result;
        self.selected_result = next_index as usize;
        changed
    }

    fn launch(&mut self) {
        let command = self.query.trim();
        if command.is_empty() {
            return;
        }

        if !command.contains(char::is_whitespace) {
            if let Some(path) = self
                .visible_results
                .get(self.selected_result)
                .and_then(|result| self.commands.get(result.index))
                .map(|entry| entry.path.clone())
            {
                self.finish_launch(launch_path(&path));
                return;
            }
        }

        self.finish_launch(launch_command(command));
    }

    fn finish_launch(&mut self, result: std::io::Result<()>) {
        match result {
            Ok(()) => self.exit = true,
            Err(error) => eprintln!("Launch failed: {error}"),
        }
    }
}

impl CompositorHandler for SparkLauncher {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        self.buffer_scale = new_factor.max(1);
        if !self.first_configure {
            self.draw();
        }
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for SparkLauncher {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.width = NonZeroU32::new(configure.new_size.0).map_or(LAUNCHER_WIDTH, NonZeroU32::get);
        self.height =
            NonZeroU32::new(configure.new_size.1).map_or(LAUNCHER_HEIGHT, NonZeroU32::get);

        if self.first_configure {
            self.first_configure = false;
        }

        self.draw();
    }
}

impl OutputHandler for SparkLauncher {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl SeatHandler for SparkLauncher {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            let keyboard = self
                .seat_state
                .get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    self.loop_handle.clone(),
                    Box::new(|state, _keyboard, event| {
                        state.handle_key_event(event, true);
                    }),
                )
                .expect("failed to create the Wayland keyboard");
            self.keyboard = Some(keyboard);
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
    }
}

impl KeyboardHandler for SparkLauncher {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
        if self.layer.wl_surface() == surface {
            self.keyboard_focus = true;
            self.draw();
        }
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        if self.layer.wl_surface() == surface {
            self.keyboard_focus = false;
            self.draw();
        }
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.handle_key_event(event, false);
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        self.handle_key_event(event, true);
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _raw_modifiers: RawModifiers,
        _layout: u32,
    ) {
        self.modifiers = modifiers;
    }
}

impl ShmHandler for SparkLauncher {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(SparkLauncher);
delegate_output!(SparkLauncher);
delegate_shm!(SparkLauncher);
delegate_seat!(SparkLauncher);
delegate_keyboard!(SparkLauncher);
delegate_layer!(SparkLauncher);
delegate_registry!(SparkLauncher);
smithay_client_toolkit::reexports::client::delegate_noop!(
    SparkLauncher: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1
);
smithay_client_toolkit::reexports::client::delegate_noop!(
    SparkLauncher: ignore wp_viewporter::WpViewporter
);
smithay_client_toolkit::reexports::client::delegate_noop!(
    SparkLauncher: ignore wp_viewport::WpViewport
);

impl ProvidesRegistryState for SparkLauncher {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for SparkLauncher {
    fn event(
        state: &mut Self,
        _proxy: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.preferred_fractional_scale = Some(scale as f64 / 120.0);
            if !state.first_configure {
                state.draw();
            }
        }
    }
}
