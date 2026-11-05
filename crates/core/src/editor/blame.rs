//! Who last changed each line, from git blame: the cursor's line says so,
//! faint, after its end, as GitLens shows it.

use super::{Editor, Effect};

impl Editor {
    /// Asks for the current file's blame when its text has changed since
    /// it was last asked for. The host waits for a pause before it runs.
    pub(crate) fn ask_blame(&mut self, fx: &mut Vec<Effect>) {
        let doc = self.doc();
        if doc.review.is_some() || doc.page.is_some() || doc.git_base.is_none() {
            return;
        }
        let Some(path) = doc.buffer.path.clone() else {
            return;
        };
        let version = doc.buffer.version();
        let key = (path.clone(), version);
        let known = self.blames.get(&path).is_some_and(|(v, _)| *v == version);
        if known || self.blame_asked.as_ref() == Some(&key) {
            return;
        }
        let text = doc.buffer.text();
        self.blame_asked = Some(key);
        fx.push(Effect::Blame {
            path,
            version,
            text,
            ignore: Vec::new(),
        });
    }

    /// The blame shown after `line` of the current file, as of its text
    /// now: who, when and the commit's summary. None for lines not yet
    /// committed.
    pub(crate) fn line_blame(&self, line: usize) -> Option<String> {
        let doc = self.doc();
        let (version, blame) = self.blames.get(doc.buffer.path.as_ref()?)?;
        if *version != doc.buffer.version() {
            return None;
        }
        let commit = blame.at(line).filter(|c| c.committed)?;
        let when = crate::git::ago(commit.time, self.now);
        Some(format!("{}, {when} · {}", commit.author, commit.summary))
    }
}
