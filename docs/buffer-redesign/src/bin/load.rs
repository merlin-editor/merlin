//! Prototype 5: opening a large file, and the memory it takes. Each way
//! runs in its own process (`load <way> <file>`) so peak memory is its own.

use std::io::BufReader;
use std::time::Instant;

use ropey::Rope;

fn peak_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines()
        .find(|l| l.starts_with("VmHWM"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|k| k.parse::<f64>().ok())
        .unwrap_or(0.0)
        / 1024.0
}

fn main() {
    let mut args = std::env::args().skip(1);
    let way = args.next().unwrap();
    let path = args.next().unwrap();
    let size = std::fs::metadata(&path).unwrap().len() as f64 / (1 << 20) as f64;
    let t = Instant::now();
    let keep: Box<dyn std::any::Any> = match way.as_str() {
        // As the app opens a file now: a String, read on the disk lane,
        // sent to the editor, which makes a rope of it; git's copy of the
        // file kept beside it for the gutter.
        "today" => {
            let text = std::fs::read_to_string(&path).unwrap();
            let rope = Rope::from_str(&text);
            let git_base = text.clone();
            drop(text);
            Box::new((rope, git_base))
        }
        "string+rope" => {
            let text = std::fs::read_to_string(&path).unwrap();
            Box::new(Rope::from_str(&text))
        }
        "streamed" => {
            let file = std::fs::File::open(&path).unwrap();
            Box::new(Rope::from_reader(BufReader::with_capacity(1 << 20, file)).unwrap())
        }
        // For viewing a file too large to load: map it, check it's text,
        // and find where its lines start, reading pages only as needed.
        "mapped" => {
            let file = std::fs::File::open(&path).unwrap();
            let map = unsafe { memmap2::Mmap::map(&file).unwrap() };
            let utf8 = std::str::from_utf8(&map).is_ok();
            // Every 1024th line's start: enough to jump anywhere fast.
            let mut marks = vec![0usize];
            let mut n = 0usize;
            for at in memchr::memchr_iter(b'\n', &map) {
                n += 1;
                if n % 1024 == 0 {
                    marks.push(at + 1);
                }
            }
            println!("  utf8 {utf8}, {n} lines, {} marks", marks.len());
            Box::new((map, marks))
        }
        _ => panic!("no way {way}"),
    };
    let ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "  {way:<12} {ms:>9.1} ms   peak {:>7.0} MB for a {size:.0} MB file ({:.2}x)",
        peak_mb(),
        peak_mb() / size
    );
    drop(keep);
}
