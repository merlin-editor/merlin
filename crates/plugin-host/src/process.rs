//! Programs a plugin started. Output is read on background threads and
//! queued; the host hands it to the plugin between editor events.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::Waker;

use crate::bindings::merlin::plugin::types::{FetchRequest, FetchResponse, Framing, ProcessEvent};
use crate::fetch;

/// Process output, or a fetch's response, on its way to a plugin.
pub(crate) struct Output {
    pub plugin: String,
    pub generation: u64,
    /// The process, or the fetch.
    pub process: u32,
    pub event: Arrival,
}

pub(crate) enum Arrival {
    Process(ProcessEvent),
    Fetched(Result<FetchResponse, String>),
    /// The plugin shared a value under this key, or took it back.
    Shared(String),
}

struct Process {
    /// The program, as the manifest names it.
    program: String,
    child: Arc<Mutex<Child>>,
    to: To,
    framing: Framing,
}

/// Where messages to a process go: its stdin, or the pipe it made, through
/// the thread that connects to it.
enum To {
    Stdin(ChildStdin),
    Pipe(Sender<Vec<u8>>),
}

/// The argument a plugin passes for a pipe the program makes, as CMake's
/// `--debugger-pipe` does: the host puts a new pipe's path there, connects
/// to it once the program has made it, and messages go through it rather
/// than stdin and stdout.
pub(crate) const PIPE: &str = "${pipe}";

/// One plugin instance's processes. Dropping it kills them.
pub(crate) struct Processes {
    plugin: String,
    generation: u64,
    tx: Sender<Output>,
    wake: Option<Waker>,
    woken: Arc<AtomicBool>,
    running: HashMap<u32, Process>,
    next_id: u32,
    open: fetch::Open,
}

impl Processes {
    pub fn new(
        plugin: String,
        generation: u64,
        tx: Sender<Output>,
        wake: Option<Waker>,
        woken: Arc<AtomicBool>,
    ) -> Self {
        Self {
            plugin,
            generation,
            tx,
            wake,
            woken,
            running: HashMap::new(),
            next_id: 0,
            open: fetch::Open::default(),
        }
    }

    /// Starts an HTTPS request to a host in `allowed`; its response
    /// arrives as output with the id this returns.
    pub fn fetch(&mut self, request: FetchRequest, allowed: Vec<String>) -> Result<u32, String> {
        fetch::check(&request, &allowed)?;
        let guard = self
            .open
            .take()
            .ok_or("8 requests are open already; wait for one to end")?;
        self.next_id += 1;
        let id = self.next_id;
        let send = self.arrivals(id);
        thread::spawn(move || {
            let response = fetch::run(request, &allowed);
            drop(guard);
            send(Arrival::Fetched(response));
        });
        Ok(id)
    }

    pub fn spawn(
        &mut self,
        program: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: &Path,
        framing: Framing,
    ) -> Result<u32, String> {
        let pipe = args.iter().any(|a| a.contains(PIPE)).then(pipe_path);
        let args: Vec<String> = match &pipe {
            Some(path) => {
                let path = path.display().to_string();
                args.iter().map(|a| a.replace(PIPE, &path)).collect()
            }
            None => args.to_vec(),
        };
        let mut command = Command::new(program);
        die_with_editor(&mut command);
        command
            .args(&args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = start(command).map_err(|e| format!("couldn't start {program}: {e}"))?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        self.next_id += 1;
        let id = self.next_id;
        let child = Arc::new(Mutex::new(child));

        let send = self.sender(id);
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                send(ProcessEvent::Stderr(line));
            }
        });

        let to = match pipe {
            Some(path) => {
                let (tx, rx) = channel::<Vec<u8>>();
                let send = self.sender(id);
                let alive = child.clone();
                thread::spawn(move || through_pipe(&path, framing, rx, &alive, send));
                To::Pipe(tx)
            }
            None => To::Stdin(stdin),
        };

        let send = self.sender(id);
        let waiter = child.clone();
        let piped = matches!(to, To::Pipe(_));
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            if piped {
                // What it prints, as it talks over the pipe.
                for line in reader.lines().map_while(Result::ok) {
                    send(ProcessEvent::Stderr(line));
                }
            } else {
                read_messages(&mut reader, framing, &send);
            }
            // Stdout closed; the process is exiting. Poll rather than wait()
            // so kill() can still take the lock.
            let code = loop {
                match waiter.lock().unwrap().try_wait() {
                    Ok(Some(status)) => break status.code(),
                    Ok(None) => {}
                    Err(_) => break None,
                }
                thread::sleep(Duration::from_millis(20));
            };
            send(ProcessEvent::Exit(code));
        });

        self.running.insert(
            id,
            Process {
                program: program.to_string(),
                child,
                to,
                framing,
            },
        );
        Ok(id)
    }

    fn sender(&self, process: u32) -> impl Fn(ProcessEvent) + Send + 'static {
        let send = self.arrivals(process);
        move |event| send(Arrival::Process(event))
    }

    fn arrivals(&self, process: u32) -> impl Fn(Arrival) + Send + 'static {
        let (tx, plugin, generation) = (self.tx.clone(), self.plugin.clone(), self.generation);
        let wake = self.wake.clone();
        let woken = self.woken.clone();
        move |event| {
            let _ = tx.send(Output {
                plugin: plugin.clone(),
                generation,
                process,
                event,
            });
            if !woken.swap(true, Ordering::AcqRel)
                && let Some(wake) = &wake
            {
                wake();
            }
        }
    }

    /// Tells the host the plugin changed what it shares under `key`, so
    /// the plugins that watch it hear.
    pub fn announce_shared(&self, key: String) {
        (self.arrivals(0))(Arrival::Shared(key));
    }

    /// The programs running, each with its process id.
    pub fn running(&self) -> Vec<(String, u32)> {
        let mut running: Vec<_> = self
            .running
            .values()
            .filter_map(|p| Some((p.program.clone(), p.child.lock().ok()?.id())))
            .collect();
        running.sort();
        running
    }

    pub fn send(&mut self, id: u32, message: &str) -> Result<(), String> {
        let process = self
            .running
            .get_mut(&id)
            .ok_or_else(|| format!("no process {id}"))?;
        let framed = match process.framing {
            Framing::Lines => format!("{message}\n"),
            Framing::ContentLength => {
                format!("Content-Length: {}\r\n\r\n{message}", message.len())
            }
        };
        match &mut process.to {
            To::Stdin(stdin) => stdin
                .write_all(framed.as_bytes())
                .and_then(|()| stdin.flush())
                .map_err(|e| format!("writing to process {id}: {e}")),
            To::Pipe(tx) => tx
                .send(framed.into_bytes())
                .map_err(|_| format!("process {id}'s pipe is closed")),
        }
    }

    pub fn kill(&mut self, id: u32) {
        if let Some(process) = self.running.remove(&id) {
            let _ = process.child.lock().unwrap().kill();
        }
    }

    /// Drops a process that has exited.
    pub fn forget(&mut self, id: u32) {
        self.running.remove(&id);
    }

    /// Hands this instance's programs to another, as a reload that keeps
    /// its state does.
    pub fn hand_over(&mut self, to: &mut Processes) {
        to.running = std::mem::take(&mut self.running);
        to.next_id = self.next_id;
    }

    pub fn kill_all(&mut self) {
        let ids: Vec<u32> = self.running.keys().copied().collect();
        for id in ids {
            self.kill(id);
        }
    }
}

impl Drop for Processes {
    fn drop(&mut self) {
        self.kill_all();
    }
}

/// Starts programs from one thread that lasts as long as the editor. Each
/// plugin calls from a thread of its own, which ends when the plugin
/// reloads, and on Linux a program is told to end when the thread that
/// started it does.
fn start(command: Command) -> io::Result<Child> {
    type Request = (Command, Sender<io::Result<Child>>);
    static STARTER: OnceLock<Mutex<Sender<Request>>> = OnceLock::new();
    let starter = STARTER.get_or_init(|| {
        let (tx, rx) = channel::<Request>();
        let _ = thread::Builder::new()
            .name("plugin programs".into())
            .spawn(move || {
                for (mut command, reply) in rx {
                    let _ = reply.send(command.spawn());
                }
            });
        Mutex::new(tx)
    });
    let (tx, rx) = channel();
    let sent = starter.lock().unwrap().send((command, tx));
    match sent {
        Ok(()) => rx
            .recv()
            .unwrap_or_else(|_| Err(io::Error::other("the starter thread ended"))),
        Err(e) => {
            let (mut command, _) = e.0;
            command.spawn()
        }
    }
}

/// Reads framed messages until the stream ends.
fn read_messages(reader: &mut impl BufRead, framing: Framing, send: &impl Fn(ProcessEvent)) {
    loop {
        let message = match framing {
            Framing::Lines => read_line(reader),
            Framing::ContentLength => read_content_length(reader),
        };
        match message {
            Ok(Some(m)) => send(ProcessEvent::Message(m)),
            Ok(None) | Err(_) => break,
        }
    }
}

/// A new pipe's path. On Unix a socket in the runtime folder, as a socket's
/// path can't be longer than about a hundred bytes; on Windows a named
/// pipe.
fn pipe_path() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let name = format!(
        "merlin-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    if cfg!(windows) {
        return PathBuf::from(format!(r"\\.\pipe\{name}"));
    }
    let short = |dir: PathBuf| (dir.as_os_str().len() < 80).then_some(dir);
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .and_then(short)
        .or_else(|| short(std::env::temp_dir()))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(format!("{name}.sock"))
}

/// Connects to the pipe at `path` once the program has made it, then
/// writes what arrives on `rx` to it and reads its messages back. Gives up
/// if the program ends first, or after a minute.
fn through_pipe(
    path: &Path,
    framing: Framing,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    alive: &Mutex<Child>,
    send: impl Fn(ProcessEvent) + Send + 'static,
) {
    let until = Instant::now() + Duration::from_secs(60);
    let (reader, mut writer) = loop {
        match connect(path) {
            Ok(ends) => break ends,
            Err(e) => {
                let ended = !matches!(alive.lock().unwrap().try_wait(), Ok(None));
                if ended || Instant::now() > until {
                    send(ProcessEvent::Stderr(format!(
                        "Couldn't connect to {}: {e}",
                        path.display()
                    )));
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    };
    thread::spawn(move || read_messages(&mut BufReader::new(reader), framing, &send));
    for message in rx {
        if writer
            .write_all(&message)
            .and_then(|()| writer.flush())
            .is_err()
        {
            break;
        }
    }
    #[cfg(unix)]
    let _ = std::fs::remove_file(path);
}

type Ends = (Box<dyn Read + Send>, Box<dyn Write + Send>);

#[cfg(unix)]
fn connect(path: &Path) -> io::Result<Ends> {
    let stream = std::os::unix::net::UnixStream::connect(path)?;
    Ok((Box::new(stream.try_clone()?), Box::new(stream)))
}

#[cfg(windows)]
fn connect(path: &Path) -> io::Result<Ends> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    Ok((Box::new(file.try_clone()?), Box::new(file)))
}

fn read_line(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\n', '\r']);
        if !line.is_empty() {
            return Ok(Some(line.to_string()));
        }
    }
}

fn read_content_length(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse().ok();
        }
    }
    let length = length.ok_or_else(|| io::Error::other("missing Content-Length"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(String::from_utf8_lossy(&body).into_owned()))
}

/// Has the process end when the editor does, however it ends, so a crash or
/// a kill leaves no language server running.
#[cfg(target_os = "linux")]
fn die_with_editor(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe extern "C" {
        fn prctl(option: i32, ...) -> i32;
    }
    const PR_SET_PDEATHSIG: i32 = 1;
    const SIGTERM: std::ffi::c_ulong = 15;
    // SAFETY: prctl is async-signal-safe, as pre_exec needs.
    unsafe {
        command.pre_exec(|| {
            prctl(PR_SET_PDEATHSIG, SIGTERM);
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn die_with_editor(_: &mut Command) {}
