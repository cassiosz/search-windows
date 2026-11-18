//! A way for a script to drive the browser you already have open, without
//! ever taking the keyboard: no synthesized input, no dependence on which
//! window is in front. The app listens on a named pipe, only when started
//! with SEARCHWIN_BENCH=1. One JSON object per line in, one per line out.
//!
//! The pipe thread never touches UI. It queues each request and calls
//! `notify` (the app posts a WM_APP to its window); the UI thread then calls
//! `drain`, and answers each request through its `Reply`, now or later.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, HLOCAL, LocalFree,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcessToken};
use windows::core::{PCWSTR, PWSTR};

/// What a script can ask for. The app decides what each one does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    State,
    Open { url: String },
    Type { text: String },
    /// A shortcut like "ctrl+l", run by the app's own handler.
    Key { keys: String },
    Eval { js: String },
    Wait {
        until: String,
        #[serde(default = "five_seconds")]
        timeout_ms: u64,
    },
    Quit,
}

fn five_seconds() -> u64 {
    5000
}

/// One answer per request: `{"ok":true,"value":..}` or `{"ok":false,"error":".."}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    pub fn ok(value: Value) -> Self {
        Response { ok: true, value: Some(value), error: None }
    }
    pub fn err(message: impl Into<String>) -> Self {
        Response { ok: false, value: None, error: Some(message.into()) }
    }
}

/// The answer to one request, owned so the app can keep it across an async
/// callback and complete it once. Dropped unanswered, the script gets an error.
pub struct Reply(Sender<Response>);

impl Reply {
    pub fn complete(self, response: Response) {
        let _ = self.0.send(response);
    }
}

type Queue = Mutex<VecDeque<(Request, Reply)>>;

/// Keeps the pipe advertised; dropping it removes the bench.pipe file.
pub struct Server {
    queue: Arc<Queue>,
    pipe: String,
}

impl Server {
    pub fn pipe_name(&self) -> &str {
        &self.pipe
    }

    /// UI thread only: hand every request queued so far to `f`.
    pub fn drain(&self, mut f: impl FnMut(Request, Reply)) {
        loop {
            let next = self.queue.lock().ok().and_then(|mut q| q.pop_front());
            match next {
                Some((request, reply)) => f(request, reply),
                None => break,
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(pipe_file());
    }
}

/// Start listening if SEARCHWIN_BENCH=1, else None and nothing is created.
/// `notify` runs on the pipe threads whenever requests are waiting.
pub fn start(notify: impl Fn() + Send + Sync + 'static) -> Option<Server> {
    if std::env::var("SEARCHWIN_BENCH").as_deref() != Ok("1") {
        return None;
    }
    start_always(notify).ok()
}

static NEXT: AtomicU32 = AtomicU32::new(0);

fn start_always(notify: impl Fn() + Send + Sync + 'static) -> std::io::Result<Server> {
    let pipe = format!(
        r"\\.\pipe\searchwin-bench-{}-{}",
        unsafe { GetCurrentProcessId() },
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let first = create_instance(&pipe, true)?;
    let file = pipe_file();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&file, &pipe)?;

    let queue: Arc<Queue> = Arc::default();
    let shared = queue.clone();
    let name = pipe.clone();
    std::thread::spawn(move || accept_loop(&name, first, shared, Arc::new(notify)));
    Ok(Server { queue, pipe })
}

/// Where the client finds the running app's pipe: beside the app's data, so
/// SEARCHWIN_DATA, which moves the one, moves the other.
pub fn pipe_file() -> PathBuf {
    if let Some(folder) = std::env::var_os("SEARCHWIN_DATA").filter(|f| !f.is_empty()) {
        return PathBuf::from(folder).join("bench.pipe");
    }
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
    base.join("SearchWin-rs").join("bench.pipe")
}

struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

type Notify = Arc<dyn Fn() + Send + Sync>;

fn accept_loop(pipe: &str, first: SendHandle, queue: Arc<Queue>, notify: Notify) {
    let mut next = Some(first);
    loop {
        let instance = match next.take() {
            Some(i) => i,
            None => match create_instance(pipe, false) {
                Ok(i) => i,
                Err(_) => return,
            },
        };
        let connected = unsafe { ConnectNamedPipe(instance.0, None) };
        if connected.is_err() && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            unsafe {
                let _ = CloseHandle(instance.0);
            }
            continue;
        }
        let file = unsafe { File::from_raw_handle(instance.0.0 as RawHandle) };
        let (queue, notify) = (queue.clone(), notify.clone());
        std::thread::spawn(move || serve(file, &queue, &*notify));
    }
}

/// A connection: each line is a request, answered in order.
fn serve(file: File, queue: &Queue, notify: &(dyn Fn() + Send + Sync)) {
    let Ok(mut out) = file.try_clone() else { return };
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { return };
        let response = match serde_json::from_str::<Request>(&line) {
            Err(e) => Response::err(format!("bad request: {e}")),
            Ok(request) => {
                let (tx, rx) = channel();
                if let Ok(mut q) = queue.lock() {
                    q.push_back((request, Reply(tx)));
                }
                notify();
                rx.recv().unwrap_or_else(|_| Response::err("the app dropped the request"))
            }
        };
        let mut text = serde_json::to_string(&response).unwrap_or_else(|_| "{\"ok\":false}".into());
        text.push('\n');
        if out.write_all(text.as_bytes()).is_err() {
            return;
        }
    }
}

/// One instance of the pipe. Only the current user may connect: the default
/// security descriptor would let Everyone read it, and a pipe that runs
/// scripts in your browser must not be open to other accounts. So the DACL is
/// protected and holds one entry, full access for this user's SID. Remote
/// machines are refused outright, and the first instance is created with
/// FIRST_PIPE_INSTANCE so nobody can squat the name before us.
fn create_instance(pipe: &str, first: bool) -> std::io::Result<SendHandle> {
    let sddl: Vec<u16> = format!("D:P(A;;GA;;;{})\0", current_user_sid()?).encode_utf16().collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .map_err(std::io::Error::other)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    let name: Vec<u16> = pipe.encode_utf16().chain(Some(0)).collect();
    let mut open = PIPE_ACCESS_DUPLEX;
    if first {
        open |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let handle = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            open,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            Some(&attributes),
        )
    };
    unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
    if handle.is_invalid() {
        return Err(std::io::Error::last_os_error());
    }
    Ok(SendHandle(handle))
}

fn current_user_sid() -> std::io::Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(std::io::Error::other)?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let got = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len);
        let sid = got.map_err(std::io::Error::other).and_then(|()| {
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut string = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut string).map_err(std::io::Error::other)?;
            let sid = string.to_string().map_err(std::io::Error::other);
            LocalFree(Some(HLOCAL(string.0.cast())));
            sid
        });
        let _ = CloseHandle(token);
        sid
    }
}

/// How long the client keeps trying to reach the app.
const CONNECT_FOR: std::time::Duration = std::time::Duration::from_secs(2);

/// Client side: send one request to the app named in bench.pipe. An app just
/// started writes the file a moment before its pipe answers, and a file left
/// by an app that died names a pipe nobody holds: both are tried again, the
/// file read afresh each time, until CONNECT_FOR runs out.
pub fn call(request: &Request) -> std::io::Result<Response> {
    let deadline = std::time::Instant::now() + CONNECT_FOR;
    let file = loop {
        let opened = std::fs::read_to_string(pipe_file()).and_then(|pipe| connect(pipe.trim()));
        match opened {
            Ok(file) => break file,
            Err(error) if std::time::Instant::now() >= deadline => return Err(error),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    };
    exchange(file, request)
}

/// Client side: send one request on a known pipe name.
pub fn call_on(pipe: &str, request: &Request) -> std::io::Result<Response> {
    exchange(connect(pipe)?, request)
}

fn connect(pipe: &str) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).open(pipe)
}

/// Only connecting is retried: a request once written is never sent twice.
fn exchange(mut file: File, request: &Request) -> std::io::Result<Response> {
    writeln!(file, "{}", serde_json::to_string(request)?)?;
    let mut line = String::new();
    BufReader::new(file).read_line(&mut line)?;
    serde_json::from_str(&line).map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;
    use std::time::Duration;

    /// The tests write their pipe file in a folder of their own, never beside
    /// the data of the app you use (a test server dropping would delete its
    /// bench.pipe). Set once, before any server starts, for every test here.
    fn own_folder() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let folder = std::env::temp_dir().join(format!("searchwin-bench-test-{}", std::process::id()));
            // SAFETY: inside call_once, before any test thread reads the variable.
            unsafe { std::env::set_var("SEARCHWIN_DATA", folder) };
        });
    }

    /// A server with a fake UI thread, the way the app does it: notify wakes
    /// the loop, the loop drains. `eval` is answered later, off-thread.
    fn fake_app() -> Arc<Server> {
        own_folder();
        let (wake, woken) = mpsc::channel::<()>();
        let wake = Mutex::new(wake);
        let server = Arc::new(
            start_always(move || {
                let _ = wake.lock().unwrap().send(());
            })
            .unwrap(),
        );
        let ui = server.clone();
        std::thread::spawn(move || {
            while woken.recv().is_ok() {
                ui.drain(|request, reply| match request {
                    Request::State => reply.complete(Response::ok(json!({"url": "about:blank"}))),
                    Request::Eval { js } => {
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(50));
                            reply.complete(Response::ok(json!(format!("ran {js}"))));
                        });
                    }
                    Request::Quit => reply.complete(Response::ok(Value::Null)),
                    other => reply.complete(Response::ok(json!(format!("{other:?}")))),
                });
            }
        });
        server
    }

    #[test]
    fn round_trips_every_command() {
        let server = fake_app();
        let pipe = server.pipe_name();
        let ask = |r: Request| call_on(pipe, &r).unwrap();
        assert_eq!(ask(Request::State).value, Some(json!({"url": "about:blank"})));
        assert!(ask(Request::Open { url: "https://a.test".into() }).ok);
        assert!(ask(Request::Type { text: "hi".into() }).ok);
        assert!(ask(Request::Key { keys: "ctrl+l".into() }).ok);
        assert!(ask(Request::Wait { until: "loaded".into(), timeout_ms: 100 }).ok);
        // Completed later, from another thread.
        assert_eq!(ask(Request::Eval { js: "1+1".into() }).value, Some(json!("ran 1+1")));
        assert!(ask(Request::Quit).ok);
        assert!(std::fs::read_to_string(pipe_file()).unwrap().contains("searchwin-bench-"));
    }

    #[test]
    fn unknown_command_is_an_error_not_a_crash() {
        let server = fake_app();
        let mut file = OpenOptions::new().read(true).write(true).open(server.pipe_name()).unwrap();
        writeln!(file, r#"{{"cmd":"dance"}}"#).unwrap();
        let mut line = String::new();
        BufReader::new(&file).read_line(&mut line).unwrap();
        let response: Response = serde_json::from_str(&line).unwrap();
        assert!(!response.ok && response.error.unwrap().contains("bad request"));
        // Still alive afterwards, on the same connection.
        writeln!(file, r#"{{"cmd":"state"}}"#).unwrap();
        line.clear();
        BufReader::new(&file).read_line(&mut line).unwrap();
        assert!(serde_json::from_str::<Response>(&line).unwrap().ok);
    }
}
