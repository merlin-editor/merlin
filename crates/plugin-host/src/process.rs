//! Programs a plugin started. Output is read on background threads and
//! queued; the host hands it to the plugin between editor events.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::Waker;

use crate::bindings::merlin::plugin::types::{Framing, ProcessEvent};

/// Process output on its way to a plugin.
pub(crate) struct Output {
    pub plugin: String,
    pub generation: u64,
    pub process: u32,
    pub event: ProcessEvent,
}

struct Process {
    /// The program, as the manifest names it.
    program: String,
    child: Arc<Mutex<Child>>,
    stdin: ChildStdin,
    framing: Framing,
}

/// One plugin instance's processes. Dropping it kills them.
pub(crate) struct Processes {
    plugin: String,
    generation: u64,
    tx: Sender<Output>,
    wake: Option<Waker>,
    running: HashMap<u32, Process>,
    next_id: u32,
}

impl Processes {
    pub fn new(plugin: String, generation: u64, tx: Sender<Output>, wake: Option<Waker>) -> Self {
        Self {
            plugin,
            generation,
            tx,
            wake,
            running: HashMap::new(),
            next_id: 0,
        }
    }

    pub fn spawn(
        &mut self,
        program: &str,
        args: &[String],
        env: &[(String, String)],
        cwd: &Path,
        framing: Framing,
    ) -> Result<u32, String> {
        let mut child = Command::new(program)
            .args(args)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("couldn't start {program}: {e}"))?;
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

        let send = self.sender(id);
        let waiter = child.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let message = match framing {
                    Framing::Lines => read_line(&mut reader),
                    Framing::ContentLength => read_content_length(&mut reader),
                };
                match message {
                    Ok(Some(m)) => send(ProcessEvent::Message(m)),
                    Ok(None) | Err(_) => break,
                }
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
                stdin,
                framing,
            },
        );
        Ok(id)
    }

    fn sender(&self, process: u32) -> impl Fn(ProcessEvent) + Send + 'static {
        let (tx, plugin, generation) = (self.tx.clone(), self.plugin.clone(), self.generation);
        let wake = self.wake.clone();
        move |event| {
            let _ = tx.send(Output {
                plugin: plugin.clone(),
                generation,
                process,
                event,
            });
            if let Some(wake) = &wake {
                wake();
            }
        }
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
        process
            .stdin
            .write_all(framed.as_bytes())
            .and_then(|()| process.stdin.flush())
            .map_err(|e| format!("writing to process {id}: {e}"))
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
