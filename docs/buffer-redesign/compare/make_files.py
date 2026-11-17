"""Makes the files the comparison opens, in files/ beside this script."""
import os, random

here = os.path.join(os.path.dirname(os.path.abspath(__file__)), "files")
os.makedirs(here, exist_ok=True)

def write(name, text):
    with open(os.path.join(here, name), "w") as f:
        f.write(text)

block = 'fn main() {\n    let x = vec![1, 2, 3];\n    println!("{x:?}");\n}\n'
write("small.rs", block * 50)
write("mid.rs", block * ((2 << 20) // len(block)))
write("big.rs", block * ((20 << 20) // len(block)))
write("one.json", "[" + '{"a":1,"b":"text"},' * ((20 << 20) // 20) + "{}]")
line = "the quick brown fox jumps over the lazy dog 0123456789\n"
write("prose.txt", line * ((85 << 20) // len(line)))
line = "2026-11-17T10:00:00Z INFO request served path=/api/x status=200 took=3ms\n"
write("big.log", line * ((342 << 20) // len(line)))

# Every line different: some editors keep repeated lines in less memory.
r = random.Random(1)
paths = ["/api/x", "/api/users", "/static/app.js", "/login", "/api/orders/{}", "/health"]
with open(os.path.join(here, "unique.log"), "w") as f:
    size, i, out = 0, 0, []
    while size < (342 << 20):
        l = "2026-11-17T{:02d}:{:02d}:{:02d}.{:06d}Z {} request served path={} status={} took={}ms id={:016x}\n".format(
            (i // 3600000) % 24, (i // 60000) % 60, (i // 1000) % 60, r.randrange(10**6),
            r.choice(["INFO", "INFO", "INFO", "WARN", "DEBUG"]), r.choice(paths).format(r.randrange(99999)),
            r.choice([200, 200, 200, 201, 304, 404, 500]), r.randrange(1, 900), r.getrandbits(64))
        out.append(l); size += len(l); i += 1
        if len(out) >= 100000:
            f.write("".join(out)); out = []
    f.write("".join(out))
print("made", sorted(os.listdir(here)))
