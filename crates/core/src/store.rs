//! The only writer of `.symbiote/`. Callers are serialized by the daemon actor.

use crate::model::{Blueprint, Evidence, GlossaryEntry, State};
use crate::util::now_iso;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const STREAM_ROTATE_BYTES: u64 = 5 * 1024 * 1024;

pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    /// Creates `.symbiote/` with mode 0700: the socket inside it is the trust boundary.
    pub fn open(root: &Path) -> io::Result<Self> {
        let dir = root.join(".symbiote");
        fs::create_dir_all(dir.join("artifacts"))?;
        fs::create_dir_all(dir.join("slices"))?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        Ok(Self { dir })
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn load_state(&self) -> State {
        fs::read_to_string(self.path("state.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save_state(&self, s: &State) -> io::Result<()> {
        write_atomic(&self.path("state.json"), &serde_json::to_vec_pretty(s)?)
    }

    pub fn save_blueprint(&self, b: &Blueprint) -> io::Result<()> {
        write_atomic(&self.path("blueprint.json"), &serde_json::to_vec_pretty(b)?)
    }

    pub fn load_blueprint(&self) -> Option<Blueprint> {
        serde_json::from_str(&fs::read_to_string(self.path("blueprint.json")).ok()?).ok()
    }

    /// Raw telemetry. Rotates to `stream.jsonl.1` at 5 MiB.
    pub fn append_stream(&self, mut line: Value) -> io::Result<()> {
        line.as_object_mut().map(|o| o.entry("ts").or_insert_with(|| now_iso().into()));
        let path = self.path("stream.jsonl");
        if fs::metadata(&path).map(|m| m.len()).unwrap_or(0) >= STREAM_ROTATE_BYTES {
            fs::rename(&path, self.path("stream.jsonl.1"))?;
        }
        append_line(&path, &line.to_string())
    }

    pub fn append_event(&self, e: &Evidence) -> io::Result<()> {
        append_line(&self.path("events.jsonl"), &serde_json::to_string(e)?)
    }

    /// Unparseable lines are skipped, never fatal: evidence is append-only history.
    pub fn load_events(&self) -> Vec<Evidence> {
        fs::read_to_string(self.path("events.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    pub fn save_slice_yaml(&self, id: &str, yaml: &str) -> io::Result<()> {
        write_atomic(&self.dir.join("slices").join(format!("{id}.yaml")), yaml.as_bytes())
    }

    pub fn load_glossary(&self) -> Vec<GlossaryEntry> {
        parse_glossary(&fs::read_to_string(self.path("glossary.md")).unwrap_or_default())
    }

    /// Adds terms not already present (case-insensitive). Returns how many were new.
    pub fn promote_terms(&self, terms: &[GlossaryEntry]) -> io::Result<usize> {
        let mut all = self.load_glossary();
        let before = all.len();
        for t in terms {
            if !all.iter().any(|e| e.term.eq_ignore_ascii_case(&t.term)) {
                all.push(t.clone());
            }
        }
        if all.len() != before {
            write_atomic(&self.path("glossary.md"), render_glossary(&all).as_bytes())?;
        }
        Ok(all.len() - before)
    }

    pub fn append_story(&self, slice_id: &str, story: &str, trigger_words: &[String]) -> io::Result<()> {
        let path = self.path("artifacts/stories.md");
        if !path.exists() {
            append_line(&path, "# Engineering Stories\n")?;
        }
        append_line(
            &path,
            &format!("## {slice_id} · {}\n{story}\n_Trigger words_: {}\n", &now_iso()[..10], trigger_words.join(", ")),
        )
    }

    /// The most recent story paragraph, for the Glossary Card and Cold Return anchor.
    pub fn latest_story(&self) -> Option<String> {
        let text = fs::read_to_string(self.path("artifacts/stories.md")).ok()?;
        let block = text.rsplit("\n## ").next()?;
        block.lines().nth(1).map(str::to_string).filter(|s| !s.is_empty())
    }
}

fn append_line(path: &Path, line: &str) -> io::Result<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")
}

/// Write-then-rename so a crash never leaves a torn state file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, path)
}

pub fn parse_glossary(text: &str) -> Vec<GlossaryEntry> {
    let mut out: Vec<GlossaryEntry> = vec![];
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("**") {
            if let Some((term, def)) = rest.split_once("**:") {
                out.push(GlossaryEntry { term: term.into(), definition: def.trim().into(), avoid: vec![] });
            }
        } else if let (Some(avoid), Some(last)) = (line.strip_prefix("_Avoid_:"), out.last_mut()) {
            last.avoid = avoid.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        }
    }
    out
}

fn render_glossary(all: &[GlossaryEntry]) -> String {
    let mut s = String::from("# Glossary\n\n");
    for e in all {
        s.push_str(&format!("**{}**: {}\n", e.term, e.definition));
        if !e.avoid.is_empty() {
            s.push_str(&format!("_Avoid_: {}\n", e.avoid.join(", ")));
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glossary_roundtrip_and_dedupe() {
        let dir = std::env::temp_dir().join(format!("symbiote-store-{}", crate::util::now_ms()));
        let st = Store::open(&dir).unwrap();
        let t = |n: &str| GlossaryEntry { term: n.into(), definition: "d: with colon".into(), avoid: vec!["x".into(), "y".into()] };
        assert_eq!(st.promote_terms(&[t("Backpressure"), t("Graceful drain")]).unwrap(), 2);
        assert_eq!(st.promote_terms(&[t("backpressure")]).unwrap(), 0);
        let g = st.load_glossary();
        assert_eq!(g.len(), 2);
        assert_eq!(g[0], t("Backpressure"));
        st.append_story("01-a", "Story one.", &["a".into()]).unwrap();
        st.append_story("02-b", "Story two.", &["b".into()]).unwrap();
        assert_eq!(st.latest_story().as_deref(), Some("Story two."));
        fs::remove_dir_all(dir).unwrap();
    }
}
