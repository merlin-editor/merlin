//! Prototype 6: what positions cost to work with: ropey by characters,
//! ropey by bytes, and crop (a rope counted in bytes), and what copying a
//! line into a Vec<char> costs against reading it in place.

use buffer_prototypes::*;

fn main() {
    for (name, text) in [("rust 20MB", rust(20)), ("log 342MB", log(342))] {
        println!("{name}");
        let mut r = time("ropey: build", || ropey::Rope::from_str(&text));
        let mut c = time("crop: build", || crop::Rope::from(text.as_str()));
        let lines = r.len_lines() - 1;
        let chars = r.len_chars();
        let bytes = r.len_bytes();
        let n = 200_000;
        each("ropey: line of a char", n, |i| { std::hint::black_box(r.char_to_line(i * 7919 % chars)); });
        each("ropey: line of a byte", n, |i| { std::hint::black_box(r.byte_to_line(i * 7919 % bytes)); });
        each("crop: line of a byte", n, |i| { std::hint::black_box(c.line_of_byte(i * 7919 % bytes)); });
        each("ropey: char where a line starts", n, |i| { std::hint::black_box(r.line_to_char(i * 7919 % lines)); });
        each("ropey: byte where a line starts", n, |i| { std::hint::black_box(r.line_to_byte(i * 7919 % lines)); });
        each("crop: byte where a line starts", n, |i| { std::hint::black_box(c.byte_of_line(i * 7919 % lines)); });
        each("ropey: char to byte", n, |i| { std::hint::black_box(r.char_to_byte(i * 7919 % chars)); });
        each("ropey: a line as Vec<char>", n, |i| {
            let v: Vec<char> = r.line(i * 7919 % lines).chars().collect();
            std::hint::black_box(v);
        });
        each("ropey: a line's chunks read in place", n, |i| {
            let s: usize = r.line(i * 7919 % lines).chunks().map(|c| c.len()).sum();
            std::hint::black_box(s);
        });
        let mut rng = Rng(3);
        each("ropey: insert by char", 100_000, |_| { let at = rng.below(r.len_chars()); r.insert(at, "x"); });
        let mut rng = Rng(3);
        each("crop: insert by byte", 100_000, |_| {
            let at = rng.below(c.byte_len());
            let at = (0..4).map(|k| at.saturating_sub(k)).find(|&b| c.is_char_boundary(b)).unwrap_or(0);
            c.insert(at, "x");
        });
        each("ropey: clone", 100_000, |_| { std::hint::black_box(r.clone()); });
        each("crop: clone", 100_000, |_| { std::hint::black_box(c.clone()); });
        time("ropey: iterate all chunks", || r.chunks().map(|c| c.len()).sum::<usize>());
        time("crop: iterate all chunks", || c.chunks().map(|c| c.len()).sum::<usize>());
    }
}
