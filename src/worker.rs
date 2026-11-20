//! Thread that owns the device. The UI sends `Request`s and receives `Event`s;
//! no I/O ever happens on the UI thread.

use crate::device::{self, Backend, Hidraw};
use crate::protocol::{self as p, Command, Status, REPORT_LEN};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

pub enum Request {
    Send(Command),
    Shutdown,
}

pub enum Event {
    Searching(String),
    Connected { name: String, version: String },
    Disconnected(String),
    Status(Status),
    Sent { label: String, frames: Vec<([u8; REPORT_LEN], [u8; REPORT_LEN])>, ok: bool },
    Log(String),
    Error(String),
}

pub struct Worker {
    tx: Sender<Event>,
    rx: Receiver<Request>,
    repaint: Box<dyn Fn() + Send>,
    demo: bool,
    seq: u8,
    /// Device opened before privileges were dropped (sudo mode), used once.
    initial: Option<Box<dyn Backend>>,
    sudo_mode: bool,
    /// True once a status report arrived in this session, i.e. the 2.4G link to the mouse is up.
    link_up: bool,
}

const ACK_TRIES: usize = 10;
const ACK_DELAY: Duration = Duration::from_millis(60);
const POKE_AFTER: Duration = Duration::from_secs(7);
/// Attempts per frame when the receiver does not acknowledge (the commands are idempotent).
const SEND_ATTEMPTS: usize = 3;
const RETRY_PAUSE: Duration = Duration::from_millis(250);
/// How long to wait for the link before sending a command right after connecting.
const LINK_WAIT: Duration = Duration::from_secs(3);
/// How long to look for a status report that confirms the change.
const VERIFY_WAIT: Duration = Duration::from_millis(1500);

impl Worker {
    pub fn spawn(
        rx: Receiver<Request>,
        tx: Sender<Event>,
        repaint: Box<dyn Fn() + Send>,
        demo: bool,
        initial: Option<Box<dyn Backend>>,
        sudo_mode: bool,
    ) {
        std::thread::Builder::new()
            .name("tmouseopen-device".into())
            .spawn(move || Worker { tx, rx, repaint, demo, seq: p::SEQ_START, initial, sudo_mode, link_up: false }.run())
            .expect("failed to spawn the device thread");
    }

    fn emit(&self, e: Event) {
        let _ = self.tx.send(e);
        (self.repaint)();
    }

    fn connect(&mut self) -> Option<Box<dyn Backend>> {
        if let Some(dev) = self.initial.take() {
            return Some(dev);
        }
        if self.demo {
            return Some(Box::new(device::Demo::new()));
        }
        let cands = device::control_candidates();
        if cands.is_empty() {
            self.emit(Event::Searching("Receiver 04f3:026f not found".into()));
            return None;
        }
        let mut last_err = String::new();
        for c in cands {
            match Hidraw::open(&c.node) {
                Ok(h) => return Some(Box::new(h)),
                Err(e) => last_err = e.to_string(),
            }
        }
        if self.sudo_mode {
            // Privileges are gone, so a replugged receiver can only be reopened via the udev rule.
            last_err = format!("{last_err} (restart with sudo -E to reconnect, or install the udev rule)");
        }
        self.emit(Event::Searching(last_err));
        None
    }

    fn run(mut self) {
        loop {
            let Some(mut dev) = self.connect() else {
                // wait before retrying, but still honor Shutdown
                for _ in 0..20 {
                    if let Ok(Request::Shutdown) | Err(TryRecvError::Disconnected) = self.rx.try_recv() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                continue;
            };
            match self.session(dev.as_mut()) {
                Ok(()) => return, // Shutdown
                Err(e) => self.emit(Event::Disconnected(e.to_string())),
            }
        }
    }

    fn poke(&self, dev: &mut dyn Backend) -> std::io::Result<String> {
        let info = dev.get_feature(p::INFO_REPORT_ID)?;
        self.emit(Event::Log(format!("info 05: {}", p::hex(&info[..8]))));
        Ok(p::parse_version(&info))
    }

    /// Ok(()) = shutdown requested; Err = the device went away.
    fn session(&mut self, dev: &mut dyn Backend) -> std::io::Result<()> {
        self.link_up = false;
        let version = self.poke(dev)?;
        self.emit(Event::Connected { name: dev.name(), version });
        let mut last_status: Option<Instant> = None;
        let mut last_poke = Instant::now();
        loop {
            match self.rx.try_recv() {
                Ok(Request::Shutdown) | Err(TryRecvError::Disconnected) => return Ok(()),
                Ok(Request::Send(cmd)) => self.send(dev, cmd)?,
                Err(TryRecvError::Empty) => {}
            }
            if let Some(data) = dev.read_input(200)? {
                if let Some(st) = Status::parse(&data) {
                    last_status = Some(Instant::now());
                    self.link_up = true;
                    self.emit(Event::Status(st));
                }
            } else if last_status.is_none_or(|t| t.elapsed() > POKE_AFTER * 2) && last_poke.elapsed() > POKE_AFTER {
                last_poke = Instant::now();
                self.emit(Event::Log("no status from the mouse — poking the receiver (move the mouse if it is asleep)".into()));
                self.poke(dev)?;
            }
        }
    }

    /// Reads input reports for up to `wait`, forwarding status reports to the UI.
    /// Returns the first status that satisfies `want`, if any.
    fn watch_status(&mut self, dev: &mut dyn Backend, wait: Duration, want: impl Fn(&Status) -> bool) -> std::io::Result<Option<Status>> {
        let start = Instant::now();
        while start.elapsed() < wait {
            if let Some(data) = dev.read_input(100)? {
                if let Some(st) = Status::parse(&data) {
                    self.link_up = true;
                    self.emit(Event::Status(st));
                    if want(&st) {
                        return Ok(Some(st));
                    }
                }
            }
        }
        Ok(None)
    }

    /// SET one frame and wait for the ack, resending the same frame if the receiver stays silent.
    fn send_frame(&mut self, dev: &mut dyn Backend, f: &[u8; REPORT_LEN]) -> std::io::Result<[u8; REPORT_LEN]> {
        let mut ack = [0u8; REPORT_LEN];
        for attempt in 1..=SEND_ATTEMPTS {
            dev.set_feature(f)?;
            for _ in 0..ACK_TRIES {
                std::thread::sleep(ACK_DELAY);
                ack = dev.get_feature(p::CMD_REPORT_ID)?;
                if ack[1] != 0 {
                    break;
                }
            }
            if ack[1] == 0x01 {
                break;
            }
            if attempt < SEND_ATTEMPTS {
                self.emit(Event::Log(format!("no ack (got {}), resending ({}/{SEND_ATTEMPTS})", p::hex(&ack[..2]), attempt + 1)));
                std::thread::sleep(RETRY_PAUSE);
            }
        }
        Ok(ack)
    }

    fn send(&mut self, dev: &mut dyn Backend, cmd: Command) -> std::io::Result<()> {
        let label = match &cmd {
            Command::Polling(hz) => format!("polling {hz} Hz"),
            Command::Dpi(_) => "DPI".to_string(),
        };
        // Right after connecting the receiver may still be bringing up the 2.4G link.
        // Wait for the first status report before sending.
        if !self.link_up {
            self.emit(Event::Log("waiting for the mouse link before sending…".into()));
            if self.watch_status(dev, LINK_WAIT, |_| true)?.is_none() {
                self.emit(Event::Log("no status yet; sending anyway".into()));
            }
        }
        let frames = match p::build(&cmd, &mut self.seq) {
            Ok(f) => f,
            Err(e) => {
                self.emit(Event::Error(e.to_string()));
                return Ok(());
            }
        };
        let mut out = Vec::new();
        let mut acked = true;
        for f in frames {
            let ack = self.send_frame(dev, &f)?;
            out.push((f, ack));
            if ack[1] != 0x01 {
                acked = false;
                break; // do not send the next part if the previous one was not acknowledged
            }
        }
        // If the ack was missed, a status report showing the new value still proves it was applied.
        let mut ok = acked;
        if !acked {
            let want: Box<dyn Fn(&Status) -> bool> = match &cmd {
                Command::Polling(hz) => {
                    let code = p::polling_code(*hz).unwrap_or(0);
                    Box::new(move |s| s.polling_code() == code)
                }
                Command::Dpi(c) => {
                    let active = c.active;
                    Box::new(move |s| s.dpi_stage() == active)
                }
            };
            if self.watch_status(dev, VERIFY_WAIT, want)?.is_some() {
                self.emit(Event::Log("no ack, but the status report confirms the change".into()));
                ok = true;
            }
        }
        self.emit(Event::Sent { label, frames: out, ok });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::DpiConfig;
    use std::sync::mpsc::channel;

    /// Simulated receiver: link comes up after `link_after`; it acks only if `acks`;
    /// applies the value only if `applies`.
    struct Flaky {
        start: Instant,
        link_after: Duration,
        acks: bool,
        applies: bool,
        status: [u8; 10],
        pending_status: bool,
        sets: usize,
    }

    impl Flaky {
        fn new(link_after_ms: u64, acks: bool, applies: bool) -> Self {
            Self {
                start: Instant::now(),
                link_after: Duration::from_millis(link_after_ms),
                acks,
                applies,
                status: [0x04, 0x01, 0x01, 0x02, 0x1e, 0, 0, 0, 0x02, 0x11],
                pending_status: false,
                sets: 0,
            }
        }
        fn up(&self) -> bool {
            self.start.elapsed() >= self.link_after
        }
    }

    impl Backend for Flaky {
        fn name(&self) -> String {
            "flaky".into()
        }
        fn set_feature(&mut self, f: &[u8; REPORT_LEN]) -> std::io::Result<()> {
            self.sets += 1;
            if f[2] == 0 {
                return Ok(()); // firmware ignores sequence 0
            }
            if self.up() && self.applies {
                match f[1] {
                    0x04 => self.status[3] = f[5],
                    0x02 => self.status[2] = f[5],
                    _ => {}
                }
                self.pending_status = true;
            }
            Ok(())
        }
        fn get_feature(&mut self, id: u8) -> std::io::Result<[u8; REPORT_LEN]> {
            let mut b = [0u8; REPORT_LEN];
            b[0] = id;
            if id == p::CMD_REPORT_ID && self.up() && self.acks {
                b[1] = 1;
            }
            Ok(b)
        }
        fn read_input(&mut self, timeout_ms: i32) -> std::io::Result<Option<Vec<u8>>> {
            if self.up() && (self.pending_status || self.start.elapsed() < self.link_after + Duration::from_millis(150)) {
                self.pending_status = false;
                return Ok(Some(self.status.to_vec()));
            }
            std::thread::sleep(Duration::from_millis(timeout_ms.clamp(0, 50) as u64));
            Ok(None)
        }
    }

    fn worker() -> (Worker, Receiver<Event>) {
        let (_req_tx, req_rx) = channel();
        let (ev_tx, ev_rx) = channel();
        let w = Worker { tx: ev_tx, rx: req_rx, repaint: Box::new(|| {}), demo: false, seq: p::SEQ_START, initial: None, sudo_mode: false, link_up: false };
        (w, ev_rx)
    }

    fn result(rx: &Receiver<Event>) -> (bool, usize) {
        rx.try_iter().find_map(|e| match e { Event::Sent { ok, frames, .. } => Some((ok, frames.len())), _ => None }).unwrap()
    }

    #[test]
    fn waits_for_link_right_after_connect() {
        // Command issued immediately; the link only comes up after 1 s.
        let (mut w, rx) = worker();
        let mut dev = Flaky::new(1000, true, true);
        w.send(&mut dev, Command::Polling(1000)).unwrap();
        assert_eq!(result(&rx), (true, 1));
        assert_eq!(dev.status[3], 0x01);
        assert_eq!(dev.sets, 1, "should have waited instead of sending into a dead link");
    }

    #[test]
    fn first_dpi_after_launch_applies() {
        // The reported bug: first DPI apply after opening the app did nothing.
        let (mut w, rx) = worker();
        w.link_up = true;
        let mut dev = Flaky::new(0, true, true);
        let cfg = DpiConfig { active: 4, ..Default::default() };
        w.send(&mut dev, Command::Dpi(cfg)).unwrap();
        assert_eq!(result(&rx), (true, 2));
        assert_eq!(dev.status[2], 4, "applied on the first try");
    }

    #[test]
    fn missing_ack_but_status_confirms() {
        let (mut w, rx) = worker();
        w.link_up = true;
        let mut dev = Flaky::new(0, false, true);
        w.send(&mut dev, Command::Polling(250)).unwrap();
        assert!(result(&rx).0);
        assert_eq!(dev.sets, SEND_ATTEMPTS, "resent before falling back to the status check");
    }

    #[test]
    fn reports_failure_when_nothing_happens() {
        let (mut w, rx) = worker();
        w.link_up = true;
        let mut dev = Flaky::new(0, false, false);
        let cfg = DpiConfig { active: 3, ..Default::default() };
        w.send(&mut dev, Command::Dpi(cfg)).unwrap();
        assert_eq!(result(&rx), (false, 1), "second DPI part must not be sent after a failed first part");
    }
}
