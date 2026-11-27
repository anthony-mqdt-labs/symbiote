//! Collaborative Workspace pane: Focus Region (Tier 1) + the pinned Phase 1 UI objects.
//! Not a chat log — every object is replaced in place by typed daemon messages.

use crate::client::Client;
use crate::terminal::TerminalView;
use futures::channel::mpsc::UnboundedReceiver;
use futures::StreamExt;
use gpui::{div, prelude::*, px, rgb, AnyElement, ClipboardItem, Context, Div, FocusHandle, FontWeight, KeyDownEvent, SharedString, Stateful, WeakEntity, Window};
use symbiote_core::model::*;

const PANEL: u32 = 0x1b1e25;
const CARD: u32 = 0x232731;
const BORDER: u32 = 0x2f3440;
const TEXT: u32 = 0xd8dce4;
const DIM: u32 = 0x8a93a6;
const ACCENT: u32 = 0x7aa2f7;
const RED: u32 = 0xf7768e;
const GREEN: u32 = 0x9ece6a;
const AMBER: u32 = 0xe0af68;
const MONO: &str = "Menlo";

struct QuizState {
    quiz: QuizView,
    /// Render-time permutation of authored option indexes (kills LLM slot-A bias).
    order: Vec<usize>,
    chosen: Option<usize>,
    result: Option<(bool, usize, Option<String>)>,
}

pub struct WorkspaceView {
    client: Client,
    terminal: WeakEntity<TerminalView>,
    offline: bool,
    mode_label: String,
    focus_text: String,
    focus_busy: bool,
    input: String,
    input_focus: FocusHandle,
    tasks: Option<TasksView>,
    picker: Option<(Vec<TrackInfo>, bool, Option<String>)>,
    glossary: GlossaryView,
    quiz: Option<QuizState>,
    connected: bool,
}

/// Fisher–Yates with a time-seeded xorshift. Not security-relevant; just unbiased placement.
pub fn shuffled(n: usize, seed: u64) -> Vec<usize> {
    let mut s = seed | 1;
    let mut v: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        v.swap(i, (s % (i as u64 + 1)) as usize);
    }
    v
}

fn seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7)
}

fn card(title: &str) -> Div {
    div().flex().flex_col().gap_2().p_3().bg(rgb(CARD)).border_1().border_color(rgb(BORDER)).rounded_md().child(div().text_xs().text_color(rgb(DIM)).font_weight(FontWeight::BOLD).child(title.to_uppercase()))
}

/// A label + wrapping text row. Flex children default to min-width:auto, which stops
/// text from wrapping; min_w_0 lets the text column shrink and wrap.
fn row(lead: impl IntoElement, text: impl IntoElement) -> Div {
    div().flex().gap_2().child(div().flex_shrink_0().child(lead)).child(div().flex_1().min_w_0().child(text))
}

fn pill(text: impl Into<SharedString>, color: u32) -> Div {
    div().px_2().py(px(1.0)).rounded_md().border_1().border_color(rgb(color)).text_color(rgb(color)).text_xs().font_weight(FontWeight::BOLD).child(text.into())
}

fn mono_block(text: &str) -> Div {
    div().flex().flex_col().p_2().bg(rgb(0x171a20)).rounded_md().font_family(MONO).text_xs().children(text.lines().map(|l| div().child(if l.is_empty() { " ".to_string() } else { l.to_string() })))
}

fn button(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, color: u32) -> Stateful<Div> {
    div().id(id.into()).px_2().py_1().rounded_md().bg(rgb(0x2b3140)).text_color(rgb(color)).text_xs().cursor_pointer().hover(|s| s.bg(rgb(0x343b4d))).child(label.into())
}

impl WorkspaceView {
    pub fn new(client: Client, mut rx: UnboundedReceiver<UiMsg>, terminal: WeakEntity<TerminalView>, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            while let Some(m) = rx.next().await {
                if this.update(cx, |this, cx| this.apply(m, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            client,
            terminal,
            offline: true,
            mode_label: "connecting…".into(),
            focus_text: String::new(),
            focus_busy: false,
            input: String::new(),
            input_focus: cx.focus_handle(),
            tasks: None,
            picker: None,
            glossary: GlossaryView::default(),
            quiz: None,
            connected: false,
        }
    }

    fn apply(&mut self, m: UiMsg, cx: &mut Context<Self>) {
        self.connected = true;
        match m {
            UiMsg::Mode { offline, label, .. } => {
                self.offline = offline;
                self.mode_label = label;
            }
            UiMsg::SetFocus { text, busy } => {
                self.focus_text = text;
                self.focus_busy = busy;
            }
            UiMsg::SetTasks(t) => {
                self.tasks = Some(t);
                self.picker = None;
            }
            UiMsg::TrackPicker { tracks, allow_custom, message } => {
                self.picker = Some((tracks, allow_custom, message));
                self.tasks = None;
            }
            UiMsg::UpsertGlossary(g) => self.glossary = g,
            UiMsg::PushQuiz { quiz } => {
                self.quiz = match (quiz, self.quiz.take()) {
                    // Same quiz re-sent (e.g. reconnect): keep its order and answer state.
                    (Some(q), Some(prev)) if prev.quiz.id == q.id => Some(prev),
                    (Some(q), _) => Some(QuizState { order: shuffled(q.options.len(), seed()), quiz: q, chosen: None, result: None }),
                    (None, _) => None,
                };
            }
            UiMsg::QuizResult { quiz_id, correct, correct_index, explanation } => {
                if let Some(q) = self.quiz.as_mut().filter(|q| q.quiz.id == quiz_id) {
                    q.result = Some((correct, correct_index, explanation));
                }
            }
        }
        cx.notify();
    }

    fn send(&self, m: ClientMsg) {
        self.client.send(&m);
    }

    fn on_input_key(&mut self, ev: &KeyDownEvent, _w: &mut Window, cx: &mut Context<Self>) {
        if self.offline {
            return;
        }
        let k = &ev.keystroke;
        match k.key.as_str() {
            "enter" => {
                let text = std::mem::take(&mut self.input);
                if !text.trim().is_empty() {
                    self.send(ClientMsg::Input { text });
                }
            }
            "backspace" => {
                if k.modifiers.alt || k.modifiers.platform {
                    self.input.clear();
                } else {
                    self.input.pop();
                }
            }
            "escape" => self.input.clear(),
            "v" if k.modifiers.platform => {
                if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    self.input.push_str(&t.replace('\n', " "));
                }
            }
            _ if k.modifiers.platform || k.modifiers.control => return,
            _ => match &k.key_char {
                Some(c) => self.input.push_str(c),
                None => return,
            },
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn header(&self) -> Div {
        let color = if self.offline { AMBER } else { GREEN };
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(div().text_color(rgb(TEXT)).font_weight(FontWeight::BOLD).child("Symbiote"))
            // The disabled input bar already reads "[offline: drill mode]"; don't say it twice.
            .child(pill(if self.offline { "Tier 0".to_string() } else { self.mode_label.clone() }, color))
    }

    fn focus_region(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.offline {
            return div()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(rgb(BORDER))
                .text_color(rgb(DIM))
                .font_family(MONO)
                .text_xs()
                .child("[offline: drill mode]")
                .into_any_element();
        }
        let focused = self.input_focus.is_focused(window);
        let mut body = div().flex().flex_col().gap_1();
        let mut in_code = false;
        let mut code = String::new();
        for line in self.focus_text.lines() {
            if line.trim_start().starts_with("```") {
                if in_code {
                    body = body.child(mono_block(&code));
                    code.clear();
                }
                in_code = !in_code;
            } else if in_code {
                code.push_str(line);
                code.push('\n');
            } else {
                body = body.child(div().child(line.to_string()));
            }
        }
        if self.focus_busy {
            body = body.child(div().text_color(rgb(DIM)).child("…"));
        }
        let placeholder = "Ask, or /hint /wait-what /mock /next /retro";
        let input = div()
            .id("focus-input")
            .track_focus(&self.input_focus)
            .on_key_down(cx.listener(Self::on_input_key))
            .on_click(cx.listener(|this, _, window, _| this.input_focus.focus(window)))
            .p_2()
            .rounded_md()
            .bg(rgb(0x171a20))
            .border_1()
            .border_color(rgb(if focused { ACCENT } else { BORDER }))
            .font_family(MONO)
            .text_xs()
            .cursor_text()
            .child(if self.input.is_empty() && !focused {
                div().text_color(rgb(DIM)).child(placeholder)
            } else {
                div().text_color(rgb(TEXT)).child(format!("{}{}", self.input, if focused { "▏" } else { "" }))
            });
        card("Focus").child(body).child(input).into_any_element()
    }

    fn picker_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (tracks, allow_custom, message) = self.picker.as_ref()?;
        let mut c = card("Pick a track").child(div().text_color(rgb(DIM)).child("One project, real toolchain, verified red→green slices."));
        if let Some(m) = message {
            c = c.child(div().text_color(rgb(AMBER)).child(m.clone()));
        }
        for (i, t) in tracks.iter().enumerate() {
            let id = t.id.clone();
            c = c.child(
                div()
                    .id(("track", i))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(if t.recommended { ACCENT } else { BORDER }))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0x2b3140)))
                    .on_click(cx.listener(move |this, _, _, _| this.send(ClientMsg::StartTrack { track: id.clone() })))
                    .child(div().font_weight(FontWeight::BOLD).child(format!("{}{}", if t.recommended { "➡️ " } else { "" }, t.title)))
                    .child(div().text_xs().text_color(rgb(DIM)).child(t.capstone_goal.clone())),
            );
        }
        if *allow_custom {
            c = c.child(div().text_xs().text_color(rgb(DIM)).child("Or describe what you want to build in the Focus input — Symbiote compiles a course for it."));
        }
        Some(c.into_any_element())
    }

    fn task_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let t = self.tasks.as_ref()?;
        let (label, color) = match t.probe_status.as_str() {
            "green" => ("GREEN", GREEN),
            "red" => ("RED", RED),
            "running" => ("RUNNING", ACCENT),
            "verifying" => ("VERIFYING", ACCENT),
            _ => ("—", DIM),
        };
        let mut c = card("Task List");
        if let Some(a) = &t.anchor {
            c = c.child(div().flex().flex_col().p_2().rounded_md().border_1().border_color(rgb(ACCENT)).text_xs().children(a.lines().map(|l| div().child(l.to_string()))));
        }
        if !t.capstone.is_empty() {
            c = c.child(div().text_xs().text_color(rgb(DIM)).child(format!("Ring {} · {} — {}", t.ring, t.ring_name, t.capstone)));
        }
        if !t.slice_id.is_empty() {
            c = c.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().flex_1().min_w_0().font_weight(FontWeight::BOLD).child(format!("{} · {}", t.slice_id, t.goal)))
                    .child(pill(label, color).flex_shrink_0()),
            );
            let probe = t.probe_cmd.clone();
            let term = self.terminal.clone();
            c = c.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().font_family(MONO).text_xs().text_color(rgb(TEXT)).child(probe.clone()))
                    .child(button("run-term", "▶ terminal", ACCENT).on_click(cx.listener(move |_, _, window, cx| {
                        let _ = term.update(cx, |t, cx| t.type_command(&probe, window, cx));
                    })))
                    .child(button("copy", "copy", DIM).on_click(cx.listener({
                        let p = t.probe_cmd.clone();
                        move |_, _, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(p.clone()))
                    }))),
            );
            let next = t.steps.iter().position(|s| !s.done);
            for (i, s) in t.steps.iter().enumerate() {
                let (mark, col) = if s.done { ("✓", GREEN) } else if Some(i) == next { ("▸", ACCENT) } else { ("○", DIM) };
                c = c.child(row(div().text_color(rgb(col)).child(mark), div().text_color(rgb(if s.done { DIM } else { TEXT })).child(s.text.clone())));
            }
            if !t.static_checks.is_empty() {
                let mut checks = div().flex().flex_col().gap_1();
                for ch in &t.static_checks {
                    let col = match ch.status.as_str() {
                        "pass" => GREEN,
                        "fail" => RED,
                        "running" => ACCENT,
                        _ => DIM,
                    };
                    checks = checks.child(div().flex().gap_2().text_xs().font_family(MONO).child(div().text_color(rgb(col)).child(ch.status.clone())).child(div().text_color(rgb(DIM)).child(ch.cmd.clone())));
                    if !ch.output_head.is_empty() {
                        checks = checks.child(mono_block(&ch.output_head.lines().take(6).collect::<Vec<_>>().join("\n")));
                    }
                }
                c = c.child(checks);
            }
        }
        if let Some(h) = &t.hint {
            c = c.child(div().flex().flex_col().gap_1().p_2().rounded_md().bg(rgb(0x2a2416)).child(div().text_xs().text_color(rgb(AMBER)).font_weight(FontWeight::BOLD).child("HINT")).child(h.clone()));
        }
        if let Some(p) = &t.stderr_pointer {
            c = c.child(mono_block(p));
        }
        for e in &t.lsp_errors {
            c = c.child(div().text_xs().font_family(MONO).text_color(rgb(AMBER)).child(format!("nvim lsp · {e}")));
        }
        if let Some(l) = &t.lesson {
            c = c.child(div().text_xs().text_color(rgb(DIM)).font_weight(FontWeight::BOLD).child("SHAPE")).child(mono_block(l));
        }
        if let Some(n) = &t.notice {
            c = c.child(div().text_color(rgb(AMBER)).child(n.clone()));
        }
        c = c.child(div().flex().gap_3().text_xs().text_color(rgb(DIM)).child(format!("due {}", t.due_items)).child(format!("open {}", t.open_misconceptions)).child(div().id("run-probe").cursor_pointer().text_color(rgb(ACCENT)).child("re-run probe").on_click(cx.listener(|this, _, _, _| this.send(ClientMsg::RunProbe)))));
        Some(c.into_any_element())
    }

    fn glossary_card(&self) -> AnyElement {
        let g = &self.glossary;
        let mut c = card("Glossary & Speech");
        if !g.trigger_words.is_empty() {
            c = c.child(div().flex().flex_wrap().gap_1().children(g.trigger_words.iter().map(|w| pill(w.clone(), ACCENT))));
        }
        for s in &g.say_this_not_that {
            c = c.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(row(div().text_color(rgb(RED)).child("Not:"), div().text_color(rgb(DIM)).child(format!("“{}”", s.avoid))))
                    .child(row(div().text_color(rgb(GREEN)).child("Say:"), format!("“{}”", s.say))),
            );
        }
        if let Some(story) = &g.story {
            c = c.child(div().flex().flex_col().gap_1().child(div().text_xs().text_color(rgb(DIM)).font_weight(FontWeight::BOLD).child("LATEST STORY")).child(div().italic().child(story.clone())));
        }
        for e in &g.terms {
            let mut row = div().flex().flex_col().child(div().child(div().font_weight(FontWeight::BOLD).child(format!("{}: ", e.term))).child(div().text_color(rgb(TEXT)).child(e.definition.clone())));
            if !e.avoid.is_empty() {
                row = row.child(div().text_xs().text_color(rgb(DIM)).child(format!("avoid: {}", e.avoid.join(", "))));
            }
            c = c.child(row);
        }
        if g.terms.is_empty() && g.trigger_words.is_empty() {
            c = c.child(div().text_color(rgb(DIM)).child("Terms are earned: they appear here when a slice goes green."));
        }
        c.into_any_element()
    }

    fn quiz_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let q = self.quiz.as_ref()?;
        let title = match q.quiz.kind.as_str() {
            "review" => "Review",
            "warmup" => "Warmup",
            _ => "Drill",
        };
        let mut c = card(title);
        if let Some(p) = &q.quiz.prompt {
            c = c.child(row(div().text_color(rgb(ACCENT)).child("Do:"), p.clone()));
        }
        c = c.child(div().font_weight(FontWeight::BOLD).child(q.quiz.question.clone()));
        for (pos, &orig) in q.order.iter().enumerate() {
            let text = q.quiz.options[orig].clone();
            let mut opt = div().id(("opt", pos)).p_2().rounded_md().border_1().child(text);
            opt = match &q.result {
                Some((_, right, _)) if orig == *right => opt.border_color(rgb(GREEN)).text_color(rgb(GREEN)),
                Some(_) if q.chosen == Some(orig) => opt.border_color(rgb(RED)).text_color(rgb(RED)),
                Some(_) => opt.border_color(rgb(BORDER)).text_color(rgb(DIM)),
                None => {
                    let id = q.quiz.id.clone();
                    opt.border_color(rgb(BORDER)).cursor_pointer().hover(|s| s.bg(rgb(0x2b3140))).on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(q) = this.quiz.as_mut().filter(|q| q.result.is_none() && q.chosen.is_none()) {
                            q.chosen = Some(orig);
                            this.send(ClientMsg::QuizAnswer { quiz_id: id.clone(), choice: orig });
                            cx.notify();
                        }
                    }))
                }
            };
            c = c.child(opt);
        }
        if let Some((correct, _, expl)) = &q.result {
            c = c.child(div().text_color(rgb(if *correct { GREEN } else { AMBER })).child(if *correct { "Correct." } else { "Not quite — the green one holds." }));
            if let Some(e) = expl {
                c = c.child(div().text_color(rgb(DIM)).child(e.clone()));
            }
            c = c.child(button("advance", "Continue →", ACCENT).on_click(cx.listener(|this, _, _, _| this.send(ClientMsg::Advance))));
        }
        Some(c.into_any_element())
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.focus_region(window, cx);
        let picker = self.picker_card(cx);
        let tasks = self.task_card(cx);
        let quiz = self.quiz_card(cx);
        let glossary = self.glossary_card();
        let mut root = div().id("workspace").flex().flex_col().size_full().gap_3().p_3().bg(rgb(PANEL)).text_color(rgb(TEXT)).text_sm().overflow_y_scroll().child(self.header()).child(focus);
        if !self.connected {
            root = root.child(div().text_color(rgb(DIM)).child("Starting the symbiote daemon…"));
        }
        // Spec order for the Phase 1 dock: Task List, Glossary & Speech, Quiz/Drill.
        root.children(picker).children(tasks).child(glossary).children(quiz)
    }
}

#[cfg(test)]
mod tests {
    use super::shuffled;

    #[test]
    fn shuffle_is_a_permutation_and_moves_slot_a() {
        let mut first_slot = [0usize; 4];
        for s in 1..2000u64 {
            let p = shuffled(4, s.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut sorted = p.clone();
            sorted.sort();
            assert_eq!(sorted, vec![0, 1, 2, 3]);
            first_slot[p[0]] += 1;
        }
        // Authored answer (index 0) must not dominate the first rendered slot.
        assert!(first_slot.iter().all(|&n| n > 300), "{first_slot:?}");
    }
}
