//! Git's diffs, split into the hunks Source Control stages one at a time,
//! and the patches that stage, unstage or discard a hunk or some of its
//! lines with `git apply`.

/// One file's part of `git diff`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// As git names it, from the repository's top.
    pub path: String,
    /// The `diff --git`, `---` and `+++` lines a patch starts with.
    pub header: String,
    pub hunks: Vec<Hunk>,
}

/// A run of changed lines with the unchanged ones around it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hunk {
    /// Where it starts in the old text and in the new, from 1, as git says.
    pub old_start: usize,
    pub new_start: usize,
    pub lines: Vec<HunkLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HunkLine {
    /// `' '` unchanged, `'+'` added, `'-'` removed.
    pub kind: char,
    pub text: String,
    /// Followed by git's "\ No newline at end of file".
    pub no_newline: bool,
}

/// A run of changed lines inside a hunk, which Source Control stages on
/// its own, as the gutter marks it: git joins changes a few lines apart
/// into one hunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// The hunk it's in, and its lines there.
    pub hunk: usize,
    pub lines: Vec<usize>,
    /// Where it is in the new text, from 1.
    pub line: usize,
    /// Its first changed line that isn't blank, trimmed.
    pub summary: String,
}

impl FileDiff {
    /// Every run of changed lines, in order.
    pub fn changes(&self) -> Vec<Change> {
        let mut out: Vec<Change> = Vec::new();
        for (h, hunk) in self.hunks.iter().enumerate() {
            let mut line = hunk.new_start.max(1);
            let mut open = false;
            for (i, l) in hunk.lines.iter().enumerate() {
                if l.kind == ' ' {
                    open = false;
                    line += 1;
                    continue;
                }
                if !open {
                    out.push(Change {
                        hunk: h,
                        lines: Vec::new(),
                        line,
                        summary: String::new(),
                    });
                    open = true;
                }
                let change = out.last_mut().expect("pushed above");
                change.lines.push(i);
                if change.summary.is_empty() && !l.text.trim().is_empty() {
                    change.summary = l.text.trim().to_string();
                }
                if l.kind == '+' {
                    line += 1;
                }
            }
        }
        // An added line says more than the removed one before it.
        for c in &mut out {
            let hunk = &self.hunks[c.hunk];
            if let Some(added) = c
                .lines
                .iter()
                .map(|&i| &hunk.lines[i])
                .find(|l| l.kind == '+' && !l.text.trim().is_empty())
            {
                c.summary = added.text.trim().to_string();
            }
        }
        out
    }

    /// A patch for `git apply` of one of [`Self::changes`].
    pub fn change_patch(&self, change: &Change, reverse: bool) -> String {
        patch(self, change.hunk, Some(&change.lines), reverse)
    }

    /// Lines added and removed in all of the file's hunks.
    pub fn counts(&self) -> (usize, usize) {
        self.hunks.iter().fold((0, 0), |(a, r), h| {
            let (ha, hr) = h.counts();
            (a + ha, r + hr)
        })
    }
}

impl Hunk {
    pub fn counts(&self) -> (usize, usize) {
        let n = |k| self.lines.iter().filter(|l| l.kind == k).count();
        (n('+'), n('-'))
    }

    /// The line it's about, from 1: its first change in the new text.
    pub fn line(&self) -> usize {
        let mut line = self.new_start.max(1);
        for l in &self.lines {
            match l.kind {
                ' ' => line += 1,
                _ => return line,
            }
        }
        line
    }

    /// A few words for its row: the first changed line that isn't blank.
    pub fn summary(&self) -> String {
        let changed = |k| {
            self.lines
                .iter()
                .find(|l| l.kind == k && !l.text.trim().is_empty())
        };
        changed('+')
            .or_else(|| changed('-'))
            .map_or_else(String::new, |l| l.text.trim().to_string())
    }
}

/// Splits the output of `git diff` into files and hunks. Binary files and
/// ones with no text changes, such as a mode change alone, have no hunks.
pub fn parse(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut in_header = false;
    for line in text.lines() {
        if line.starts_with("diff --git ") {
            files.push(FileDiff {
                path: path_of(line),
                header: format!("{line}\n"),
                hunks: Vec::new(),
            });
            in_header = true;
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("@@ ") {
            in_header = false;
            let (old_start, new_start) = starts(rest);
            file.hunks.push(Hunk {
                old_start,
                new_start,
                lines: Vec::new(),
            });
            continue;
        }
        if in_header {
            if let Some(path) = line.strip_prefix("+++ b/") {
                file.path = unquote(path);
            }
            file.header.push_str(line);
            file.header.push('\n');
            continue;
        }
        let Some(hunk) = file.hunks.last_mut() else {
            continue;
        };
        if line.starts_with('\\') {
            if let Some(last) = hunk.lines.last_mut() {
                last.no_newline = true;
            }
            continue;
        }
        let mut chars = line.chars();
        let kind = match chars.next() {
            Some(k @ (' ' | '+' | '-')) => k,
            // An empty context line, from a diff whose trailing spaces were
            // trimmed.
            None => ' ',
            Some(_) => continue,
        };
        hunk.lines.push(HunkLine {
            kind,
            text: chars.as_str().to_string(),
            no_newline: false,
        });
    }
    files
}

/// The path in `diff --git a/x b/x`: the new one, `b/`'s.
fn path_of(line: &str) -> String {
    let rest = line.trim_start_matches("diff --git ");
    match rest.rfind(" b/") {
        Some(i) => unquote(&rest[i + 3..]),
        None => rest.to_string(),
    }
}

fn unquote(path: &str) -> String {
    path.trim_matches('"').to_string()
}

/// `-18,4 +18,6 @@ fn shade` gives (18, 18).
fn starts(rest: &str) -> (usize, usize) {
    let mut parts = rest.split_whitespace();
    let num = |p: Option<&str>| {
        p.and_then(|p| p[1..].split(',').next()?.parse().ok())
            .unwrap_or(0)
    };
    let old = num(parts.next());
    let new = num(parts.next());
    (old, new)
}

/// A patch for `git apply` of hunk `index` of `file`, with only the lines
/// in `chosen` (indices into the hunk's lines) when given, else all of it.
///
/// Going forward (staging), a change left out stays as it is in the old
/// text: an added line is dropped and a removed one kept. In `reverse`
/// (unstaging or discarding, which apply with `-R`), the new text is what's
/// there now, so a change left out stays as it is in the new text instead.
pub fn patch(file: &FileDiff, index: usize, chosen: Option<&[usize]>, reverse: bool) -> String {
    let Some(hunk) = file.hunks.get(index) else {
        return String::new();
    };
    let mut lines: Vec<(char, &HunkLine)> = Vec::new();
    for (i, line) in hunk.lines.iter().enumerate() {
        let taken = chosen.is_none_or(|c| c.contains(&i));
        let kind = match (line.kind, taken, reverse) {
            (' ', ..) | (_, true, _) => line.kind,
            ('+', false, false) | ('-', false, true) => continue,
            _ => ' ',
        };
        lines.push((kind, line));
    }
    let old_len = lines.iter().filter(|(k, _)| *k != '+').count();
    let new_len = lines.iter().filter(|(k, _)| *k != '-').count();
    let mut out = file.header.clone();
    out.push_str(&format!(
        "@@ -{},{old_len} +{},{new_len} @@\n",
        hunk.old_start, hunk.new_start
    ));
    for (kind, line) in lines {
        out.push(kind);
        out.push_str(&line.text);
        out.push('\n');
        if line.no_newline {
            out.push_str("\\ No newline at end of file\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
diff --git a/src/palette.rs b/src/palette.rs
index 1111111..2222222 100644
--- a/src/palette.rs
+++ b/src/palette.rs
@@ -1,3 +1,4 @@
+/// A colour a few steps from the accent.
 pub fn shade(accent: u32, step: u32) -> u32 {
-    accent + step
+    accent + step * 2
 }
@@ -5,4 +6,4 @@ pub fn shade(accent: u32, step: u32) -> u32 {
 pub fn theme_colours(accent: u32) -> Vec<u32> {
-    let steps = [1u32, 2, 3];
+    let steps = [1u32, 2, 3, 4];
     steps.iter().map(|s| shade(accent, *s)).collect()
 }
diff --git a/notes.txt b/notes.txt
index 3333333..4444444 100644
--- a/notes.txt
+++ b/notes.txt
@@ -1 +1 @@
-old
\\ No newline at end of file
+new
\\ No newline at end of file
";

    #[test]
    fn a_diff_splits_into_files_and_hunks() {
        let files = parse(DIFF);
        assert_eq!(files.len(), 2);
        let f = &files[0];
        assert_eq!(f.path, "src/palette.rs");
        assert_eq!(f.hunks.len(), 2);
        assert_eq!(f.counts(), (3, 2));
        assert_eq!(f.hunks[0].line(), 1);
        assert_eq!(f.hunks[1].line(), 7);
        assert_eq!(
            f.hunks[0].summary(),
            "/// A colour a few steps from the accent."
        );
        assert!(f.header.ends_with("+++ b/src/palette.rs\n"));
        assert!(files[1].hunks[0].lines[0].no_newline);
    }

    #[test]
    fn a_hunk_patch_has_the_header_and_its_counts() {
        let files = parse(DIFF);
        let p = patch(&files[0], 1, None, false);
        assert!(p.starts_with("diff --git a/src/palette.rs"));
        assert!(p.contains("@@ -5,4 +6,4 @@\n pub fn theme_colours"));
    }

    #[test]
    fn some_lines_stage_and_the_rest_stay_as_they_were() {
        let files = parse(DIFF);
        // Only the doc comment, from the first hunk: the changed line stays
        // as it was in the old text.
        let p = patch(&files[0], 0, Some(&[0]), false);
        assert!(
            p.ends_with(
                "@@ -1,3 +1,4 @@\n+/// A colour a few steps from the accent.\n \
                 pub fn shade(accent: u32, step: u32) -> u32 {\n     accent + step\n }\n"
            ),
            "{p}"
        );
        // Unstaging only the doc comment keeps the other change staged.
        let p = patch(&files[0], 0, Some(&[0]), true);
        assert!(
            p.ends_with(
                "@@ -1,3 +1,4 @@\n+/// A colour a few steps from the accent.\n \
                 pub fn shade(accent: u32, step: u32) -> u32 {\n     accent + step * 2\n }\n"
            ),
            "{p}"
        );
    }

    #[test]
    fn changes_are_the_runs_of_changed_lines_in_each_hunk() {
        let files = parse(DIFF);
        let changes = files[0].changes();
        let at: Vec<(usize, usize, &str)> = changes
            .iter()
            .map(|c| (c.hunk, c.line, c.summary.as_str()))
            .collect();
        assert_eq!(
            at,
            [
                (0, 1, "/// A colour a few steps from the accent."),
                (0, 3, "accent + step * 2"),
                (1, 7, "let steps = [1u32, 2, 3, 4];"),
            ]
        );
        let p = files[0].change_patch(&changes[1], false);
        assert!(
            p.ends_with(
                "@@ -1,3 +1,3 @@\n pub fn shade(accent: u32, step: u32) -> u32 {\n\
                 -    accent + step\n+    accent + step * 2\n }\n"
            ),
            "{p}"
        );
    }

    #[test]
    fn a_missing_newline_at_the_end_is_kept() {
        let files = parse(DIFF);
        let p = patch(&files[1], 0, None, false);
        assert!(
            p.ends_with("-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n")
        );
    }
}
