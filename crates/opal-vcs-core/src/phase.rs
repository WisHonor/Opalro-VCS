//! Throwaway timers (branch gate/m10-timers, never merged): parts timed, counted and summed on
//! any thread, and each flush and rename in the order they happen, printed to stderr at the end
//! of the command when `OPALRO_TIMERS` is set. One line each, tab-separated, starting `@T`.
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("OPALRO_TIMERS").is_some())
}

pub struct Timer {
    name: &'static str,
    ns: AtomicU64,
    calls: AtomicU64,
    listed: AtomicBool,
}

static TIMERS: Mutex<Vec<&'static Timer>> = Mutex::new(Vec::new());

impl Timer {
    pub const fn new(name: &'static str) -> Timer {
        Timer {
            name,
            ns: AtomicU64::new(0),
            calls: AtomicU64::new(0),
            listed: AtomicBool::new(false),
        }
    }

    pub fn time<T>(&'static self, f: impl FnOnce() -> T) -> T {
        if !on() {
            return f();
        }
        let began = Instant::now();
        let out = f();
        self.add(began.elapsed());
        out
    }

    pub fn add(&'static self, took: Duration) {
        self.ns.fetch_add(took.as_nanos() as u64, Relaxed);
        self.calls.fetch_add(1, Relaxed);
        if !self.listed.swap(true, Relaxed) {
            TIMERS.lock().unwrap().push(self);
        }
    }
}

/// `timed!("name", expr)`: `expr`, its time added to the part `name`. No `?` or `return` inside.
#[macro_export]
macro_rules! timed {
    ($name:expr, $e:expr) => {{
        static PHASE_TIMER: $crate::phase::Timer = $crate::phase::Timer::new($name);
        PHASE_TIMER.time(|| $e)
    }};
}

struct Event {
    kind: &'static str,
    what: String,
    ns: u64,
}

static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());

/// `f`, kept as one `kind` event ("flush", "rename") named `what`, in the order they happen.
pub fn event<T>(kind: &'static str, what: impl FnOnce() -> String, f: impl FnOnce() -> T) -> T {
    if !on() {
        return f();
    }
    let began = Instant::now();
    let out = f();
    let ns = began.elapsed().as_nanos() as u64;
    let what = what();
    EVENTS.lock().unwrap().push(Event { kind, what, ns });
    out
}

/// A name as it is shown: Opalro's random temporary names all alike, so runs line up.
pub fn shown(name: &std::ffi::OsStr) -> String {
    let name = name.to_string_lossy();
    if name.starts_with("tmp-") {
        let accented = name.ends_with(crate::fsio::ACCENTED_SUFFIX);
        return if accented { "tmp-*(accented)".into() } else { "tmp-*".into() };
    }
    if name.ends_with(".pack") {
        return "<pack>.pack".into();
    }
    name.into_owned()
}

/// Prints every part used and every event, then the whole command's time.
pub fn report(whole: Duration) {
    if !on() {
        return;
    }
    let ms = |ns: u64| ns as f64 / 1e6;
    let mut out = String::new();
    let line = |out: &mut String, s: String| {
        out.push_str(&s);
        out.push('\n');
    };
    line(
        &mut out,
        format!("@T\tpart\twhole command (main, first line to last)\t1\t{:.4}", ms(whole.as_nanos() as u64)),
    );
    for t in TIMERS.lock().unwrap().iter() {
        let (ns, calls) = (t.ns.load(Relaxed), t.calls.load(Relaxed));
        line(&mut out, format!("@T\tpart\t{}\t{calls}\t{:.4}", t.name, ms(ns)));
    }
    let events = EVENTS.lock().unwrap();
    for kind in ["flush", "rename"] {
        let of: Vec<&Event> = events.iter().filter(|e| e.kind == kind).collect();
        let total: u64 = of.iter().map(|e| e.ns).sum();
        let label = if kind == "flush" { "flushes, all" } else { "renames, all" };
        line(&mut out, format!("@T\tpart\t{label}\t{}\t{:.4}", of.len(), ms(total)));
        for (i, e) in of.iter().enumerate() {
            line(&mut out, format!("@T\t{kind}\t{}\t{}\t{:.4}", i + 1, e.what, ms(e.ns)));
        }
    }
    let _ = std::io::stderr().lock().write_all(out.as_bytes());
}
