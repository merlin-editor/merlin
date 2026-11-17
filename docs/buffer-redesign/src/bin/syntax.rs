//! Prototype 8: tree-sitter on a large file, as a highlighting plugin
//! would run it: parsing from the rope's chunks, what the tree costs, and
//! colours for a screen asked for when it's drawn, against colours for the
//! whole file kept.

use buffer_prototypes::*;
use ropey::Rope;
use streaming_iterator::StreamingIterator;
use tree_sitter::{InputEdit, Parser, Point, Query, QueryCursor};

fn tokens(query: &Query, tree: &tree_sitter::Tree, text: &[u8], range: Option<std::ops::Range<usize>>) -> usize {
    let mut cursor = QueryCursor::new();
    if let Some(r) = range {
        cursor.set_byte_range(r);
    }
    let mut caps = cursor.captures(query, tree.root_node(), text);
    let mut n = 0;
    while caps.next().is_some() {
        n += 1;
    }
    n
}

fn main() {
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let query = Query::new(&lang, tree_sitter_rust::HIGHLIGHTS_QUERY).unwrap();
    // Real files given as arguments, else made-up ones of 2 and 20 MB.
    let files: Vec<(String, String)> = match std::env::args().skip(1).collect::<Vec<_>>() {
        paths if !paths.is_empty() => paths
            .into_iter()
            .map(|p| {
                let name = p.rsplit('/').take(2).collect::<Vec<_>>().join(" in ");
                (name, std::fs::read_to_string(&p).unwrap())
            })
            .collect(),
        _ => [2usize, 20].iter().map(|&mb| (format!("made-up {mb}MB"), rust(mb))).collect(),
    };
    for (name, text) in files {
        let mut rope = Rope::from_str(&text);
        println!("{name}: {:.1} MB, {} lines", text.len() as f64 / (1 << 20) as f64, rope.len_lines());
        let mut parser = Parser::new();
        parser.set_language(&lang).unwrap();
        // `WINDOW=1`: only the window around the screen, so its memory is
        // counted on its own.
        if std::env::var("WINDOW").is_ok() {
            let mid_line = rope.len_lines() / 2;
            let (a, b) = (rope.line_to_byte(mid_line - 1000), rope.line_to_byte(mid_line + 1000));
            let before = rss_mb();
            let part = time("parse 2 000 lines around the screen", || parser.parse(&text[a..b], None).unwrap());
            println!("  {:<48} {:>10.1} MB", "that tree's memory (RSS growth)", rss_mb() - before);
            drop(part);
            continue;
        }
        let before = rss_mb();
        let tree = time("parse from a String", || parser.parse(&text, None).unwrap());
        println!("  {:<48} {:>10.1} MB", "tree's memory (RSS growth)", rss_mb() - before);
        drop(tree);
        let mut tree = time("parse from the rope's chunks", || {
            parser
                .parse_with_options(
                    &mut |byte, _| {
                        if byte >= rope.len_bytes() {
                            return &[][..];
                        }
                        let (chunk, start, _, _) = rope.chunk_at_byte(byte);
                        &chunk.as_bytes()[byte - start..]
                    },
                    None,
                    None,
                )
                .unwrap()
        });
        let bytes = text.as_bytes();
        let all = time("colours for the whole file", || tokens(&query, &tree, bytes, None));
        println!("  {all} captures in all");
        let screen = |line: usize| rope.line_to_byte(line)..rope.line_to_byte(line + 60);
        let lines = rope.len_lines() - 70;
        each("colours for one screen (60 lines)", 200, |i| {
            std::hint::black_box(tokens(&query, &tree, bytes, Some(screen(i * 7919 % lines))));
        });
        // Typing: an edit, the tree told, and parsed again from it.
        // Inside the name of a function past the middle, as typing in one.
        let mid = rope.len_bytes() / 2;
        let at = mid + text[mid..].find("fn ").expect("a function") + 4;
        let line = rope.byte_to_line(at);
        let col = at - rope.line_to_byte(line);
        let mut changed = String::new();
        let t = std::time::Instant::now();
        let n = 50;
        for k in 0..n {
            rope.insert(rope.byte_to_char(at + k), "x");
            tree.edit(&InputEdit {
                start_byte: at + k,
                old_end_byte: at + k,
                new_end_byte: at + k + 1,
                start_position: Point::new(line, col + k),
                old_end_position: Point::new(line, col + k),
                new_end_position: Point::new(line, col + k + 1),
            });
            let new = parser
                .parse_with_options(
                    &mut |byte, _| {
                        if byte >= rope.len_bytes() {
                            return &[][..];
                        }
                        let (chunk, start, _, _) = rope.chunk_at_byte(byte);
                        &chunk.as_bytes()[byte - start..]
                    },
                    Some(&tree),
                    None,
                )
                .unwrap();
            let ranges: Vec<_> = tree.changed_ranges(&new).collect();
            if k == n - 1 {
                changed = format!("{} changed range(s)", ranges.len());
            }
            tree = new;
        }
        let per = t.elapsed().as_secs_f64() * 1e3 / n as f64;
        println!("  {:<48} {per:>10.3} ms each ({changed})", "a keystroke: edit, parse again");
        // Only a window of lines around the screen parsed, for files too
        // large to keep a tree for: its time and its tree's memory.
        drop(tree);
        let mid_line = rope.len_lines() / 2;
        let (a, b) = (rope.line_to_byte(mid_line - 1000), rope.line_to_byte(mid_line + 1000));
        let mut window = Parser::new();
        window.set_language(&lang).unwrap();
        let before = rss_mb();
        let part = time("parse 2 000 lines around the screen", || window.parse(&text[a..b], None).unwrap());
        println!("  {:<48} {:>10.1} MB", "that tree's memory (RSS growth)", rss_mb() - before);
        each("colours for one screen of it", 200, |i| {
            let at = (i * 7919) % 1900;
            let s = rope.line_to_byte(mid_line - 1000 + at) - a;
            let e = rope.line_to_byte(mid_line - 1000 + at + 60) - a;
            std::hint::black_box(tokens(&query, &part, &bytes[a..b], Some(s..e)));
        });
        // Colours kept for all of it, as today's layout, against the screen's.
        println!(
            "  {:<48} {:>10.1} MB",
            "all colours kept, 24 B a token (today)",
            all as f64 * 24.0 / (1 << 20) as f64
        );
        println!(
            "  {:<48} {:>10.1} MB",
            "all colours kept, 3 B a token (runs)",
            all as f64 * 3.0 / (1 << 20) as f64
        );
    }
}
