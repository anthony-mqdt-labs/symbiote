//! Symbiote desktop app: terminal pane (60%) + Collaborative Workspace pane (40%).
//! Usage: symbiote-app [PROJECT_DIR]   (default: $SYMBIOTE_ROOT, then cwd)

mod client;
mod terminal;
mod workspace;

use gpui::{div, prelude::*, px, relative, rgb, size, App, Application, Bounds, Context, Entity, TitlebarOptions, Window, WindowBounds, WindowOptions};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use symbiote_core::daemon::{resolve_root, sock_path};
use symbiote_core::util::{init_log, log};
use terminal::TerminalView;
use workspace::WorkspaceView;

struct Root {
    terminal: Entity<TerminalView>,
    workspace: Entity<WorkspaceView>,
}

impl Root {
    fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = symbiote_core::store::Store::open(&root).expect("cannot create .symbiote/");
        let shell_dir = symbiote_core::shell::install(&store.dir).expect("cannot write shell integration");
        let bin = client::daemon_bin();
        let user_zdotdir = std::env::var("ZDOTDIR").ok().filter(|z| !z.contains(".symbiote")).unwrap_or_else(|| std::env::var("HOME").unwrap_or_default());
        let env: HashMap<String, String> = [
            ("ZDOTDIR", shell_dir.to_string_lossy().to_string()),
            ("SYMBIOTE_SHELL_DIR", shell_dir.to_string_lossy().to_string()),
            ("SYMBIOTE_USER_ZDOTDIR", user_zdotdir),
            ("SYMBIOTE_ACTIVE", "1".into()),
            ("SYMBIOTE_ROOT", root.to_string_lossy().to_string()),
            ("SYMBIOTE_BIN", bin.to_string_lossy().to_string()),
            ("SYMBIOTE_SOCK", sock_path(&root).to_string_lossy().to_string()),
            ("TERM", "xterm-256color".into()),
            ("COLORTERM", "truecolor".into()),
            ("PATH", symbiote_core::verify::augmented_path()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        let terminal = cx.new(|cx| TerminalView::new(root.clone(), env, window, cx));
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let client = client::Client::start(root, tx);
        let weak = terminal.downgrade();
        let workspace = cx.new(|cx| WorkspaceView::new(client, rx, weak, cx));
        Self { terminal, workspace }
    }
}

impl Render for Root {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .bg(rgb(terminal::BG))
            .child(div().w(relative(0.6)).h_full().child(self.terminal.clone()))
            .child(div().w(relative(0.4)).h_full().border_l_1().border_color(rgb(0x2f3440)).child(self.workspace.clone()))
    }
}

fn main() {
    init_log("symbiote-app");
    let started = std::time::Instant::now();
    let root = resolve_root(std::env::args().nth(1).map(PathBuf::from));
    if let Err(e) = std::fs::create_dir_all(&root) {
        eprintln!("symbiote-app: cannot create {}: {e}", root.display());
        std::process::exit(1);
    }
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    log("info", "process_start", json!({"exe": std::env::args().next(), "ppid": unsafe { libc_getppid() }, "cwd": std::env::current_dir().ok(), "root": root}));
    let title = format!("Symbiote — {}", root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
        let opts = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }), ..Default::default() };
        if let Err(e) = cx.open_window(opts, |window, cx| cx.new(|cx| Root::new(root.clone(), window, cx))) {
            log("critical", "window_open_failed", json!({"message": e.to_string()}));
            cx.quit();
        }
        cx.on_window_closed(move |cx| {
            log("info", "process_exit", json!({"exit_code": 0, "wall_ms": started.elapsed().as_millis() as u64, "peak_rss_bytes": symbiote_core::util::peak_rss_bytes()}));
            cx.quit();
        })
        .detach();
        cx.activate(true);
    });
}

extern "C" {
    #[link_name = "getppid"]
    fn libc_getppid() -> i32;
}
