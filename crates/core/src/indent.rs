//! What one level of indentation is in a file: a tab, or some spaces. VS
//! Code guesses it from the file and shows it in the status bar; so does
//! merlin.

use ropey::Rope;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indent {
    pub tabs: bool,
    /// Spaces a level takes, or the columns a tab counts as.
    pub width: usize,
}

impl Indent {
    pub const fn spaces(width: usize) -> Self {
        Self { tabs: false, width }
    }

    /// One level, as text.
    pub fn unit(&self) -> String {
        if self.tabs {
            "\t".into()
        } else {
            " ".repeat(self.width)
        }
    }

    /// The text that indents to `column`, from column 0.
    pub fn to_column(&self, column: usize) -> String {
        if self.tabs {
            "\t".repeat(column / self.width) + &" ".repeat(column % self.width)
        } else {
            " ".repeat(column)
        }
    }

    /// The status bar's name for it.
    pub fn label(&self) -> String {
        if self.tabs {
            format!("Tab Size: {}", self.width)
        } else {
            format!("Spaces: {}", self.width)
        }
    }
}

/// Lines looked at when guessing; enough for any file's habits.
const SAMPLE: usize = 2000;

/// Guesses a file's indentation from how its lines start, as VS Code does:
/// tabs if more lines start with one than with spaces, else the change in
/// leading spaces that's most common between neighbouring lines. `None`
/// when nothing in the file is indented.
pub fn detect(rope: &Rope) -> Option<Indent> {
    let (mut tab_lines, mut space_lines) = (0usize, 0usize);
    // How often each step, 2 to 8 spaces, is the change from the line
    // before.
    let mut steps = [0usize; 9];
    let mut previous = 0usize;
    for line in rope.lines().take(SAMPLE) {
        let mut chars = line.chars().peekable();
        match chars.peek() {
            Some('\t') => {
                tab_lines += 1;
                continue;
            }
            Some(' ') => space_lines += 1,
            _ => {}
        }
        let spaces = line.chars().take_while(|c| *c == ' ').count();
        // Blank lines say nothing about indentation.
        if line.chars().skip(spaces).all(char::is_whitespace) {
            continue;
        }
        let step = spaces.abs_diff(previous);
        if (2..=8).contains(&step) {
            steps[step] += 1;
        }
        previous = spaces;
    }
    if tab_lines == 0 && space_lines == 0 {
        return None;
    }
    if tab_lines > space_lines {
        return Some(Indent {
            tabs: true,
            width: 4,
        });
    }
    // Ties go to the smaller step: a file of 2s also steps by 4 at times.
    let width = (2..=8)
        .rev()
        .max_by_key(|&w| steps[w])
        .filter(|&w| steps[w] > 0)?;
    Some(Indent::spaces(width))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guess(text: &str) -> Option<Indent> {
        detect(&Rope::from_str(text))
    }

    #[test]
    fn guesses_spaces_by_their_step() {
        let two = "a {\n  b {\n    c\n  }\n}\n";
        assert_eq!(guess(two), Some(Indent::spaces(2)));
        let four = "fn a() {\n    if b {\n        c();\n    }\n}\n";
        assert_eq!(guess(four), Some(Indent::spaces(4)));
    }

    #[test]
    fn guesses_tabs_when_most_lines_start_with_one() {
        let text = "func a() {\n\tb()\n\tif c {\n\t\td()\n\t}\n}\n";
        assert!(guess(text).unwrap().tabs);
    }

    #[test]
    fn says_nothing_about_flat_files() {
        assert_eq!(guess("a\nb\n\nc\n"), None);
    }

    #[test]
    fn indents_to_a_column_with_tabs_then_spaces() {
        let tabs = Indent {
            tabs: true,
            width: 4,
        };
        assert_eq!(tabs.to_column(9), "\t\t ");
        assert_eq!(Indent::spaces(2).to_column(3), "   ");
    }
}
