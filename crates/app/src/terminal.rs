//! Terminal pane: alacritty_terminal owns the PTY + VT state; GPUI paints the grid.
//! Grading never runs through here — this is the learner's real shell.

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::StreamExt;
use gpui::{
    canvas, div, fill, font, point, prelude::*, px, size, Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, FocusHandle, Font, FontStyle, FontWeight, Hsla, KeyDownEvent,
    Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Rgba, ScrollWheelEvent, SharedString, TextRun, UTF16Selection, UnderlineStyle, Window,
};
use std::ops::Range;
use serde_json::json;
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use symbiote_core::util::log;

/// First installed wins; Nerd Fonts first so prompt glyphs (starship, p10k) render.
const FONT_CANDIDATES: [&str; 4] = ["FiraCode Nerd Font", "JetBrainsMono Nerd Font", "MesloLGS NF", "Menlo"];
const FONT_SIZE: f32 = 13.0;
const LINE_HEIGHT: f32 = 1.3;
pub const BG: u32 = 0x14161b;
const FG: u32 = 0xd8dce4;
const CURSOR: u32 = 0x7aa2f7;
const SELECTION: u32 = 0x33467c;

/// The 16 ANSI colors (normal, then bright).
const ANSI: [u32; 16] = [
    0x1d2027, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5, 0x414868, 0xff8fa3, 0xb9f27c, 0xffc777, 0x8db0ff, 0xc7a9ff, 0xa4daff, 0xe6ebff,
];

#[derive(Clone)]
pub struct Listener(UnboundedSender<TermEvent>);

impl EventListener for Listener {
    fn send_event(&self, e: TermEvent) {
        let _ = self.0.unbounded_send(e);
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct GridSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Measured once per paint; shared with mouse handlers to map pixels → cells.
#[derive(Clone, Copy, Default)]
struct Metrics {
    origin: gpui::Point<Pixels>,
    cell_w: Pixels,
    cell_h: Pixels,
    size: Option<GridSize>,
}

pub struct TerminalView {
    term: Arc<FairMutex<Term<Listener>>>,
    pty: EventLoopSender,
    metrics: Rc<Cell<Metrics>>,
    focus: FocusHandle,
    exited: Option<String>,
    selecting: bool,
    /// Button held while a full-screen app has mouse reporting on (xterm code).
    mouse_btn: Option<u8>,
    last_mouse_cell: Option<(usize, usize)>,
    /// IME preedit (e.g. pinyin before commit); painted at the cursor, never sent to the PTY.
    marked: Option<String>,
    scroll_acc: f32,
    root: PathBuf,
    env: HashMap<String, String>,
    font_family: SharedString,
}

type Started = (Arc<FairMutex<Term<Listener>>>, EventLoopSender, UnboundedReceiver<TermEvent>);

fn start(root: &Path, env: &HashMap<String, String>, g: GridSize) -> std::io::Result<Started> {
    let (tx, rx) = unbounded();
    let listener = Listener(tx);
    let term = Arc::new(FairMutex::new(Term::new(Config::default(), &g, listener.clone())));
    let opts = tty::Options {
        shell: Some(tty::Shell::new("/bin/zsh".into(), vec!["-l".into()])),
        working_directory: Some(root.to_path_buf()),
        drain_on_exit: false,
        env: env.clone(),
    };
    let pty = tty::new(&opts, WindowSize { num_lines: g.rows as u16, num_cols: g.cols as u16, cell_width: 8, cell_height: 17 }, 0)?;
    let el = EventLoop::new(term.clone(), listener, pty, false, false)?;
    let sender = el.channel();
    el.spawn();
    log("info", "worker_spawned", json!({"worker": "pty", "backend": "zsh", "cols": g.cols, "rows": g.rows}));
    Ok((term, sender, rx))
}

impl TerminalView {
    pub fn new(root: PathBuf, env: HashMap<String, String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (term, pty, rx) = start(&root, &env, GridSize { cols: 100, rows: 40 }).expect("failed to open a PTY");
        let focus = cx.focus_handle();
        focus.focus(window);
        let installed = window.text_system().all_font_names();
        let font_family: SharedString = std::env::var("SYMBIOTE_FONT")
            .ok()
            .into_iter()
            .chain(FONT_CANDIDATES.iter().map(|s| s.to_string()))
            .find(|f| installed.iter().any(|i| i == f))
            .unwrap_or_else(|| "Menlo".into())
            .into();
        log("info", "font_selected", json!({"family": font_family.to_string()}));
        let view = Self { term, pty, metrics: Rc::new(Cell::new(Metrics::default())), focus, exited: None, selecting: false, mouse_btn: None, last_mouse_cell: None, marked: None, scroll_acc: 0.0, root, env, font_family };
        Self::pump(rx, cx);
        view
    }

    fn pump(mut rx: UnboundedReceiver<TermEvent>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.next().await {
                if this.update(cx, |this, cx| this.on_term_event(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.pty.send(Msg::Input(bytes.into()));
    }

    /// Used by the Task List's "Run in terminal": types the probe like the learner would.
    pub fn type_command(&mut self, cmd: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.term.lock().scroll_display(Scroll::Bottom);
        self.write(format!("{cmd}\r").into_bytes());
        self.focus.focus(window);
        cx.notify();
    }

    fn on_term_event(&mut self, ev: TermEvent, cx: &mut Context<Self>) {
        match ev {
            TermEvent::Wakeup | TermEvent::MouseCursorDirty | TermEvent::CursorBlinkingChange => cx.notify(),
            TermEvent::PtyWrite(s) => self.write(s.into_bytes()),
            TermEvent::ClipboardStore(_, s) => cx.write_to_clipboard(ClipboardItem::new_string(s)),
            TermEvent::ClipboardLoad(_, f) => {
                let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
                self.write(f(&text).into_bytes());
            }
            TermEvent::ColorRequest(i, f) => {
                let colors = *self.term.lock().colors();
                let rgb = color_rgb(&colors, Color::Indexed(i.min(255) as u8), i);
                self.write(f(rgb).into_bytes());
            }
            TermEvent::TextAreaSizeRequest(f) => {
                let m = self.metrics.get();
                let g = m.size.unwrap_or(GridSize { cols: 100, rows: 40 });
                self.write(f(WindowSize { num_lines: g.rows as u16, num_cols: g.cols as u16, cell_width: f32::from(m.cell_w) as u16, cell_height: f32::from(m.cell_h) as u16 }).into_bytes());
            }
            TermEvent::ChildExit(status) => {
                log("info", "worker_exited", json!({"worker": "pty", "exit_code": status.code(), "reason": "child_exit"}));
                self.exited = Some(format!("[shell exited ({}) — press Enter to restart]", status.code().map_or("signal".into(), |c| c.to_string())));
                cx.notify();
            }
            TermEvent::Exit => {
                self.exited.get_or_insert_with(|| "[shell exited — press Enter to restart]".into());
                cx.notify();
            }
            TermEvent::Title(_) | TermEvent::ResetTitle | TermEvent::Bell => {}
        }
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        let g = self.metrics.get().size.unwrap_or(GridSize { cols: 100, rows: 40 });
        match start(&self.root, &self.env, g) {
            Ok((term, pty, rx)) => {
                self.term = term;
                self.pty = pty;
                self.exited = None;
                Self::pump(rx, cx);
            }
            Err(e) => log("error", "worker_spawn_failed", json!({"worker": "pty", "message": e.to_string()})),
        }
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        if self.exited.is_some() {
            if k.key == "enter" {
                self.restart(cx);
            }
            return cx.stop_propagation();
        }
        if k.modifiers.platform {
            match k.key.as_str() {
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        let bracketed = self.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
                        self.write(paste_bytes(&text, bracketed));
                    }
                }
                "c" => {
                    if let Some(s) = self.term.lock().selection_to_string() {
                        cx.write_to_clipboard(ClipboardItem::new_string(s));
                    }
                }
                "k" => self.write(&b"\x0c"[..]),
                _ => return, // let app-level shortcuts (cmd-q, cmd-w) through
            }
            return cx.stop_propagation();
        }
        // Plain text goes to the macOS input context so IMEs can compose; it comes back through
        // replace_text_in_range. Held keys are written here so repeat (`jjjj` in nvim) beats the
        // press-and-hold accent popup.
        if is_plain_text(k) && self.marked.is_none() && !ev.is_held {
            return;
        }
        let app_cursor = self.term.lock().mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = key_bytes(k, app_cursor) {
            let mut t = self.term.lock();
            t.selection = None;
            t.scroll_display(Scroll::Bottom);
            drop(t);
            self.write(bytes);
            cx.stop_propagation();
        }
    }

    /// Viewport (col, row, right_half) under a pixel position.
    fn viewport_cell(&self, pos: gpui::Point<Pixels>) -> Option<(usize, usize, bool)> {
        let m = self.metrics.get();
        let g = m.size?;
        let x = ((pos.x - m.origin.x) / m.cell_w).max(0.0);
        let y = ((pos.y - m.origin.y) / m.cell_h).max(0.0);
        Some(((x.floor() as usize).min(g.cols - 1), (y.floor() as usize).min(g.rows - 1), x.fract() >= 0.5))
    }

    fn cell_at(&self, pos: gpui::Point<Pixels>) -> Option<(GridPoint, Side)> {
        let (col, row, right) = self.viewport_cell(pos)?;
        let offset = self.term.lock().grid().display_offset();
        Some((GridPoint::new(Line(row as i32 - offset as i32), Column(col)), if right { Side::Right } else { Side::Left }))
    }

    /// The app's mouse mode, unless Shift is held (Shift forces local selection, like iTerm/Alacritty).
    fn mouse_mode(&self, mods: &Modifiers) -> Option<TermMode> {
        let mode = *self.term.lock().mode();
        (mode.intersects(TermMode::MOUSE_MODE) && !mods.shift).then_some(mode)
    }

    fn report_mouse(&mut self, mode: TermMode, button: u8, mods: &Modifiers, motion: bool, pos: gpui::Point<Pixels>, pressed: bool) {
        let Some((col, row, _)) = self.viewport_cell(pos) else { return };
        if motion && self.last_mouse_cell == Some((col, row)) {
            return;
        }
        self.last_mouse_cell = Some((col, row));
        if let Some(b) = mouse_report(button, mods, motion, col, row, pressed, mode.contains(TermMode::SGR_MOUSE)) {
            self.write(b);
        }
    }

    fn on_button_down(&mut self, ev: &MouseDownEvent, window: &mut Window, _cx: &mut Context<Self>) {
        self.focus.focus(window);
        if let Some(mode) = self.mouse_mode(&ev.modifiers) {
            let b = button_code(ev.button);
            self.mouse_btn = Some(b);
            self.report_mouse(mode, b, &ev.modifiers, false, ev.position, true);
        }
    }

    fn on_button_up(&mut self, ev: &MouseUpEvent, _w: &mut Window, _cx: &mut Context<Self>) {
        if let (Some(mode), Some(b)) = (self.mouse_mode(&ev.modifiers), self.mouse_btn.take()) {
            self.report_mouse(mode, b, &ev.modifiers, false, ev.position, false);
        }
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
        if self.mouse_mode(&ev.modifiers).is_some() {
            return self.on_button_down(ev, window, cx);
        }
        if let Some((p, side)) = self.cell_at(ev.position) {
            let ty = if ev.click_count >= 2 { SelectionType::Semantic } else { SelectionType::Simple };
            self.term.lock().selection = Some(Selection::new(ty, p, side));
            self.selecting = true;
            cx.notify();
        }
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _w: &mut Window, cx: &mut Context<Self>) {
        if let Some(mode) = self.mouse_mode(&ev.modifiers) {
            match self.mouse_btn {
                Some(b) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => self.report_mouse(mode, b, &ev.modifiers, true, ev.position, true),
                None if mode.contains(TermMode::MOUSE_MOTION) => self.report_mouse(mode, 3, &ev.modifiers, true, ev.position, true),
                _ => {}
            }
            return;
        }
        if !self.selecting || ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        if let Some((p, side)) = self.cell_at(ev.position) {
            if let Some(sel) = self.term.lock().selection.as_mut() {
                sel.update(p, side);
            }
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, w: &mut Window, cx: &mut Context<Self>) {
        if self.mouse_btn.is_some() {
            return self.on_button_up(ev, w, cx);
        }
        self.selecting = false;
        let mut t = self.term.lock();
        if t.selection.as_ref().is_some_and(|s| s.is_empty()) {
            t.selection = None;
        }
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let m = self.metrics.get();
        if m.cell_h <= px(0.0) {
            return;
        }
        self.scroll_acc += ev.delta.pixel_delta(m.cell_h).y / m.cell_h;
        let lines = self.scroll_acc.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_acc -= lines as f32;
        if let Some(mode) = self.mouse_mode(&ev.modifiers) {
            for _ in 0..lines.unsigned_abs() {
                self.last_mouse_cell = None;
                self.report_mouse(mode, if lines > 0 { 64 } else { 65 }, &ev.modifiers, false, ev.position, true);
            }
            return;
        }
        let mode = *self.term.lock().mode();
        if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            // Full-screen apps (less, nvim) get arrow keys, like every native terminal.
            let seq: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            self.write(seq.repeat(lines.unsigned_abs() as usize));
        } else {
            self.term.lock().scroll_display(Scroll::Delta(lines));
        }
        cx.notify();
    }
}

/// One styled run of cells painted at its own column, so wide glyphs never drift the grid.
struct Run {
    col: usize,
    row: usize,
    /// Grid cells covered; differs from char count for wide and combining characters.
    cols: usize,
    /// A wide glyph's advance may not be exactly 2 cells, so nothing is appended after it.
    wide: bool,
    text: String,
    fg: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
}

struct Frame {
    rects: Vec<(usize, usize, usize, Hsla)>,
    runs: Vec<Run>,
    cursor: Option<(usize, usize, CursorShape)>,
    banner: Option<String>,
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let term = self.term.clone();
        let pty = self.pty.clone();
        let metrics = self.metrics.clone();
        let paint_metrics = self.metrics.clone();
        let focused = self.focus.is_focused(window);
        let banner = self.exited.clone();
        let family = self.font_family.clone();
        let family2 = self.font_family.clone();
        let marked = self.marked.clone();
        let entity = cx.entity();
        let focus = self.focus.clone();
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_button_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_button_down))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_button_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_button_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .size_full()
            .bg(gpui::rgb(BG))
            .p(px(6.0))
            .child(
                canvas(
                    move |bounds, window, _cx| {
                        let fs = px(FONT_SIZE);
                        let ts = window.text_system();
                        let fid = ts.resolve_font(&font(family));
                        let cell_w = ts.advance(fid, fs, 'm').map(|s| s.width).unwrap_or(px(8.0));
                        let cell_h = px((FONT_SIZE * LINE_HEIGHT).round());
                        let g = GridSize { cols: ((bounds.size.width / cell_w).floor() as usize).max(2), rows: ((bounds.size.height / cell_h).floor() as usize).max(2) };
                        let mut m = metrics.get();
                        if m.size != Some(g) {
                            term.lock().resize(g);
                            let _ = pty.send(Msg::Resize(WindowSize { num_lines: g.rows as u16, num_cols: g.cols as u16, cell_width: f32::from(cell_w) as u16, cell_height: f32::from(cell_h) as u16 }));
                        }
                        m = Metrics { origin: bounds.origin, cell_w, cell_h, size: Some(g) };
                        metrics.set(m);
                        build_frame(&term.lock(), focused, banner)
                    },
                    move |bounds, frame: Frame, window, cx| {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                        let m = paint_metrics.get();
                        let at = |col: usize, row: usize| point(bounds.origin.x + m.cell_w * col as f32, bounds.origin.y + m.cell_h * row as f32);
                        for (col, row, n, color) in &frame.rects {
                            window.paint_quad(fill(Bounds::new(at(*col, *row), size(m.cell_w * *n as f32, m.cell_h)), *color));
                        }
                        if let Some((col, row, shape)) = frame.cursor {
                            let o = at(col, row);
                            let c = gpui::rgb(CURSOR);
                            let b = match shape {
                                CursorShape::Beam => Bounds::new(o, size(px(2.0), m.cell_h)),
                                CursorShape::Underline => Bounds::new(point(o.x, o.y + m.cell_h - px(2.0)), size(m.cell_w, px(2.0))),
                                _ if focused => Bounds::new(o, size(m.cell_w, m.cell_h)),
                                _ => Bounds::new(point(o.x, o.y + m.cell_h - px(2.0)), size(m.cell_w, px(2.0))),
                            };
                            window.paint_quad(fill(b, Hsla::from(c).opacity(if matches!(shape, CursorShape::Block) && focused { 0.55 } else { 1.0 })));
                        }
                        let base = font(family2);
                        for r in &frame.runs {
                            let f = Font { weight: if r.bold { FontWeight::BOLD } else { FontWeight::NORMAL }, style: if r.italic { FontStyle::Italic } else { FontStyle::Normal }, ..base.clone() };
                            let run = TextRun {
                                len: r.text.len(),
                                font: f,
                                color: r.fg,
                                background_color: None,
                                underline: r.underline.then(|| UnderlineStyle { thickness: px(1.0), color: Some(r.fg), wavy: false }),
                                strikethrough: None,
                            };
                            let line = window.text_system().shape_line(SharedString::from(r.text.clone()), px(FONT_SIZE), &[run], None);
                            let _ = line.paint(at(r.col, r.row), m.cell_h, window, cx);
                        }
                        if let (Some(text), Some((col, row, _))) = (marked, frame.cursor) {
                            let run = TextRun { len: text.len(), font: base.clone(), color: gpui::rgb(FG).into(), background_color: None, underline: Some(UnderlineStyle { thickness: px(1.0), color: Some(gpui::rgb(CURSOR).into()), wavy: false }), strikethrough: None };
                            let line = window.text_system().shape_line(SharedString::from(text), px(FONT_SIZE), &[run], None);
                            window.paint_quad(fill(Bounds::new(at(col, row), size(line.width, m.cell_h)), gpui::rgb(SELECTION)));
                            let _ = line.paint(at(col, row), m.cell_h, window, cx);
                        }
                        if let Some(text) = frame.banner {
                            let run = TextRun { len: text.len(), font: base.clone(), color: gpui::rgb(0xe0af68).into(), background_color: None, underline: None, strikethrough: None };
                            let line = window.text_system().shape_line(SharedString::from(text), px(FONT_SIZE), &[run], None);
                            let row = m.size.map_or(0, |g| g.rows.saturating_sub(1));
                            window.paint_quad(fill(Bounds::new(at(0, row), size(bounds.size.width, m.cell_h)), gpui::rgb(0x2a2016)));
                            let _ = line.paint(at(0, row), m.cell_h, window, cx);
                        }
                    },
                )
                .size_full(),
            )
    }
}

/// IME bridge. The PTY has no editable document, so the only "text" is the preedit; committed
/// text is written straight to the shell.
impl EntityInputHandler for TerminalView {
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, _w: &mut Window, _cx: &mut Context<Self>) -> Option<String> {
        let m = self.marked.as_deref()?;
        let utf16: Vec<u16> = m.encode_utf16().collect();
        let r = range.start.min(utf16.len())..range.end.min(utf16.len());
        *adjusted = Some(r.clone());
        Some(String::from_utf16_lossy(&utf16[r]))
    }

    fn selected_text_range(&mut self, _ignore_disabled: bool, _w: &mut Window, _cx: &mut Context<Self>) -> Option<UTF16Selection> {
        let n = self.marked.as_deref().map_or(0, |m| m.encode_utf16().count());
        Some(UTF16Selection { range: n..n, reversed: false })
    }

    fn marked_text_range(&self, _w: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_deref().map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _w: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(&mut self, _range: Option<Range<usize>>, text: &str, _w: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        if !text.is_empty() && self.exited.is_none() {
            let mut t = self.term.lock();
            t.selection = None;
            t.scroll_display(Scroll::Bottom);
            drop(t);
            self.write(text.to_string().into_bytes());
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(&mut self, _range: Option<Range<usize>>, text: &str, _sel: Option<Range<usize>>, _w: &mut Window, cx: &mut Context<Self>) {
        self.marked = (!text.is_empty()).then(|| text.to_string());
        cx.notify();
    }

    /// Where the IME candidate window goes: the cursor cell.
    fn bounds_for_range(&mut self, _range: Range<usize>, _element: Bounds<Pixels>, _w: &mut Window, _cx: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let m = self.metrics.get();
        let t = self.term.lock();
        let cur = t.grid().cursor.point;
        let row = cur.line.0 + t.grid().display_offset() as i32;
        Some(Bounds::new(point(m.origin.x + m.cell_w * cur.column.0 as f32, m.origin.y + m.cell_h * row.max(0) as f32), size(m.cell_w, m.cell_h)))
    }

    fn character_index_for_point(&mut self, _p: gpui::Point<Pixels>, _w: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        None
    }
}

/// Printable text with no Ctrl/Alt: the IME's job, not ours.
fn is_plain_text(k: &Keystroke) -> bool {
    !k.modifiers.control && !k.modifiers.alt && !k.modifiers.platform && k.key_char.as_deref().is_some_and(|c| !c.is_empty() && !c.chars().any(char::is_control))
}

fn button_code(b: MouseButton) -> u8 {
    match b {
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        _ => 0,
    }
}

/// xterm mouse report. `button`: 0 left, 1 middle, 2 right, 3 none (motion), 64/65 wheel.
fn mouse_report(button: u8, mods: &Modifiers, motion: bool, col: usize, row: usize, pressed: bool, sgr: bool) -> Option<Vec<u8>> {
    let b = button + 4 * u8::from(mods.shift) + 8 * u8::from(mods.alt) + 16 * u8::from(mods.control) + 32 * u8::from(motion);
    if sgr {
        return Some(format!("\x1b[<{b};{};{}{}", col + 1, row + 1, if pressed { 'M' } else { 'm' }).into_bytes());
    }
    // ponytail: legacy X10 encoding caps at col/row 223; UTF8_MOUSE (1005) not implemented — SGR covers modern apps.
    if col > 222 || row > 222 {
        return None;
    }
    let b = if pressed { b } else { (b & !3) | 3 };
    Some(vec![0x1b, b'[', b'M', 32 + b, 33 + col as u8, 33 + row as u8])
}

fn build_frame(term: &Term<Listener>, focused: bool, banner: Option<String>) -> Frame {
    let content = term.renderable_content();
    let colors = content.colors;
    let offset = content.display_offset as i32;
    let default_bg = gpui::rgb(BG);
    let mut frame = Frame { rects: vec![], runs: vec![], cursor: None, banner };
    let sel = content.selection;
    let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && offset == 0;
    let cur = content.cursor;
    if show_cursor && cur.shape != CursorShape::Hidden {
        let row = cur.point.line.0 + offset;
        if row >= 0 {
            frame.cursor = Some((cur.point.column.0, row as usize, if focused { cur.shape } else { CursorShape::HollowBlock }));
        }
    }
    for ind in content.display_iter {
        let cell = ind.cell;
        let row = ind.point.line.0 + offset;
        if row < 0 || cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let (row, col) = (row as usize, ind.point.column.0);
        let mut fg = color_rgb(colors, cell.fg, 256);
        let mut bg = color_rgb(colors, cell.bg, 257);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        let mut fg_h: Hsla = rgba(fg).into();
        if cell.flags.contains(Flags::DIM) {
            fg_h = fg_h.opacity(0.6);
        }
        if cell.flags.contains(Flags::HIDDEN) {
            fg_h = rgba(bg).into();
        }
        let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
        let selected = sel.as_ref().is_some_and(|s| s.contains(ind.point));
        let bg_h: Hsla = if selected { gpui::rgb(SELECTION).into() } else { rgba(bg).into() };
        if bg_h != Hsla::from(default_bg) {
            match frame.rects.last_mut() {
                Some((c, r, n, color)) if *r == row && *c + *n == col && *color == bg_h => *n += width,
                _ => frame.rects.push((col, row, width, bg_h)),
            }
        }
        let ch = if cell.c == '\0' || cell.c == '\t' { ' ' } else { cell.c };
        if ch == ' ' && !cell.flags.intersects(Flags::ALL_UNDERLINES) {
            continue; // blank cells only need their background
        }
        let (bold, italic, underline) = (cell.flags.contains(Flags::BOLD), cell.flags.contains(Flags::ITALIC), cell.flags.intersects(Flags::ALL_UNDERLINES));
        let wide = width == 2;
        let mut text = ch.to_string();
        if let Some(zw) = cell.zerowidth() {
            text.extend(zw);
        }
        match frame.runs.last_mut() {
            Some(r) if !wide && !r.wide && r.row == row && r.col + r.cols == col && r.fg == fg_h && (r.bold, r.italic, r.underline) == (bold, italic, underline) => {
                r.text.push_str(&text);
                r.cols += 1;
            }
            _ => frame.runs.push(Run { col, row, cols: width, wide, text, fg: fg_h, bold, italic, underline }),
        }
    }
    frame
}

fn rgba(c: Rgb) -> Rgba {
    Rgba { r: c.r as f32 / 255.0, g: c.g as f32 / 255.0, b: c.b as f32 / 255.0, a: 1.0 }
}

fn hex(v: u32) -> Rgb {
    Rgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }
}

/// Resolve a cell color: program-set palette entries first, then our theme / xterm-256.
fn color_rgb(colors: &Colors, c: Color, fallback_idx: usize) -> Rgb {
    match c {
        Color::Spec(rgb) => rgb,
        Color::Indexed(i) => colors[i as usize].unwrap_or_else(|| xterm256(i)),
        Color::Named(n) => colors[n].unwrap_or_else(|| named(n, fallback_idx)),
    }
}

fn named(n: NamedColor, _fallback: usize) -> Rgb {
    let i = n as usize;
    match n {
        NamedColor::Foreground | NamedColor::BrightForeground => hex(FG),
        NamedColor::DimForeground => hex(0x9aa1ae),
        NamedColor::Background => hex(BG),
        NamedColor::Cursor => hex(CURSOR),
        _ if i < 16 => hex(ANSI[i]),
        _ if (259..=266).contains(&i) => {
            let c = hex(ANSI[i - 259]);
            Rgb { r: c.r / 3 * 2, g: c.g / 3 * 2, b: c.b / 3 * 2 }
        }
        _ => hex(FG),
    }
}

fn xterm256(i: u8) -> Rgb {
    match i {
        0..=15 => hex(ANSI[i as usize]),
        16..=231 => {
            let v = i - 16;
            let s = |x: u8| if x == 0 { 0 } else { 55 + 40 * x };
            Rgb { r: s(v / 36), g: s((v / 6) % 6), b: s(v % 6) }
        }
        _ => {
            let g = 8 + 10 * (i - 232);
            Rgb { r: g, g, b: g }
        }
    }
}

/// Bracketed paste with embedded end-markers stripped, so pasted text can't break out
/// of paste mode and inject keystrokes.
fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if !bracketed {
        return text.replace("\r\n", "\r").replace('\n', "\r").into_bytes();
    }
    let clean = text.replace("\x1b[201~", "").replace('\x1b', "");
    format!("\x1b[200~{clean}\x1b[201~").into_bytes()
}

/// GPUI keystroke → bytes a VT100/xterm program expects.
pub fn key_bytes(k: &Keystroke, app_cursor: bool) -> Option<Vec<u8>> {
    let m = &k.modifiers;
    let code = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.control);
    let csi = |final_: char| -> Vec<u8> {
        if code > 1 {
            format!("\x1b[1;{code}{final_}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{final_}").into_bytes()
        } else {
            format!("\x1b[{final_}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> { if code > 1 { format!("\x1b[{n};{code}~") } else { format!("\x1b[{n}~") }.into_bytes() };
    let alt = |b: Vec<u8>| if m.alt { [vec![0x1b], b].concat() } else { b };
    Some(match k.key.as_str() {
        "enter" => alt(b"\r".to_vec()),
        "tab" if m.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "escape" => b"\x1b".to_vec(),
        "backspace" if m.control => b"\x08".to_vec(),
        "backspace" => alt(b"\x7f".to_vec()),
        "space" if m.control => vec![0],
        "space" => alt(b" ".to_vec()),
        "up" => csi('A'),
        "down" => csi('B'),
        "right" => csi('C'),
        "left" => csi('D'),
        "home" => csi('H'),
        "end" => csi('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        f if f.starts_with('f') && f.len() <= 3 => {
            let n: u8 = f[1..].parse().ok()?;
            tilde(*[0u8, 0, 0, 0, 0, 15, 17, 18, 19, 20, 21, 23, 24].get(n as usize)?)
        }
        key if m.control && key.len() == 1 => {
            let c = key.as_bytes()[0].to_ascii_lowercase();
            let b = match c {
                b'a'..=b'z' => c - b'a' + 1,
                b'@' | b'2' => 0,
                b'[' | b'3' => 0x1b,
                b'\\' | b'4' => 0x1c,
                b']' | b'5' => 0x1d,
                b'^' | b'6' => 0x1e,
                b'_' | b'-' | b'7' => 0x1f,
                b'/' => 0x1f,
                _ => return None,
            };
            alt(vec![b])
        }
        // Option acts as Meta: ESC + the base key, the convention shells and nvim expect.
        key if m.alt && key.chars().count() == 1 => {
            let base = if m.shift { key.to_uppercase() } else { key.to_string() };
            [vec![0x1b], base.into_bytes()].concat()
        }
        _ => k.key_char.as_ref()?.clone().into_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(s: &str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    #[test]
    fn keys_map_to_vt_sequences() {
        assert_eq!(key_bytes(&ks("enter"), false).unwrap(), b"\r");
        assert_eq!(key_bytes(&ks("ctrl-c"), false).unwrap(), [3]);
        assert_eq!(key_bytes(&ks("up"), false).unwrap(), b"\x1b[A");
        assert_eq!(key_bytes(&ks("up"), true).unwrap(), b"\x1bOA", "DECCKM app-cursor mode");
        assert_eq!(key_bytes(&ks("shift-left"), false).unwrap(), b"\x1b[1;2D");
        assert_eq!(key_bytes(&ks("alt-b"), false).unwrap(), b"\x1bb");
        assert_eq!(key_bytes(&ks("f5"), false).unwrap(), b"\x1b[15~");
        assert_eq!(key_bytes(&ks("shift-tab"), false).unwrap(), b"\x1b[Z");
    }

    #[test]
    fn mouse_reports() {
        let none = Modifiers::default();
        assert_eq!(mouse_report(0, &none, false, 4, 9, true, true).unwrap(), b"\x1b[<0;5;10M");
        assert_eq!(mouse_report(0, &none, false, 4, 9, false, true).unwrap(), b"\x1b[<0;5;10m");
        assert_eq!(mouse_report(0, &none, true, 0, 0, true, true).unwrap(), b"\x1b[<32;1;1M", "drag adds 32");
        assert_eq!(mouse_report(65, &Modifiers { control: true, ..none }, false, 0, 0, true, true).unwrap(), b"\x1b[<81;1;1M");
        assert_eq!(mouse_report(2, &none, false, 0, 0, true, false).unwrap(), [0x1b, b'[', b'M', 34, 33, 33]);
        assert_eq!(mouse_report(2, &none, false, 0, 0, false, false).unwrap(), [0x1b, b'[', b'M', 35, 33, 33], "X10 release is button 3");
        assert!(mouse_report(0, &none, false, 300, 0, true, false).is_none());
    }

    #[test]
    fn plain_text_goes_to_ime() {
        // macOS fills key_char on real events; parse() leaves it empty.
        let typed = |s: &str, c: &str| Keystroke { key_char: Some(c.into()), ..ks(s) };
        assert!(is_plain_text(&typed("a", "a")));
        assert!(is_plain_text(&typed("shift-a", "A")));
        assert!(!is_plain_text(&typed("ctrl-a", "a")));
        assert!(!is_plain_text(&typed("alt-a", "å")));
        assert!(!is_plain_text(&typed("enter", "\r")));
        assert!(!is_plain_text(&ks("up")));
    }

    #[test]
    fn paste_cannot_escape_bracketed_mode() {
        let evil = "ls\x1b[201~rm -rf ~\n";
        let out = String::from_utf8(paste_bytes(evil, true)).unwrap();
        assert_eq!(out.matches("\x1b[201~").count(), 1);
        assert!(out.ends_with("\x1b[201~"));
    }
}
