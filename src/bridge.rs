use crate::{
    c0c::Pair,
    core::{self, Direction, Group, Parity},
    recorder::{Record, Sink},
};
use serialport::{DataBits, FlowControl, SerialPort, StopBits};
use std::{
    io::{self, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Debug)]
pub enum State {
    Running,
    Waiting(String),
    Stopping,
    Stopped,
    Error(String),
}

#[derive(Default)]
pub struct OutputStats {
    pub sent: AtomicU64,
    pub dropped: AtomicU64,
}
pub struct Stats {
    pub rx: AtomicU64,
    pub tx: AtomicU64,
    pub display_dropped: AtomicU64,
    pub tx_unconfirmed: AtomicU64,
    pub warning: Mutex<Option<String>>,
    pub outputs: Vec<(String, Arc<OutputStats>)>,
}
pub struct DisplayEvent {
    pub group: u32,
    pub ms: i64,
    pub direction: Direction,
    pub source: Option<String>,
    pub raw: Vec<u8>,
    pub notice: Option<String>,
}
struct Command {
    source: String,
    raw: Vec<u8>,
}

pub struct Runtime {
    pub state: Arc<Mutex<State>>,
    pub stats: Arc<Stats>,
    stop: Arc<AtomicBool>,
    exiting: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    stopped: Option<Receiver<()>>,
}

fn idle(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}
fn cancelled(stop: &AtomicBool, attempt: &AtomicBool) -> bool {
    stop.load(Ordering::Relaxed) || attempt.load(Ordering::Relaxed)
}
fn wait_stop(stop: &AtomicBool, duration: Duration) {
    let start = Instant::now();
    while !stop.load(Ordering::Relaxed) && start.elapsed() < duration {
        thread::sleep(Duration::from_millis(20));
    }
}

fn open_physical(group: &Group) -> serialport::Result<Box<dyn SerialPort>> {
    serialport::new(&group.port, group.baud)
        .data_bits(match group.data_bits {
            5 => DataBits::Five,
            6 => DataBits::Six,
            7 => DataBits::Seven,
            _ => DataBits::Eight,
        })
        .parity(match group.parity {
            Parity::None => serialport::Parity::None,
            Parity::Even => serialport::Parity::Even,
            Parity::Odd => serialport::Parity::Odd,
        })
        .stop_bits(if group.stop_bits == 2 {
            StopBits::Two
        } else {
            StopBits::One
        })
        .flow_control(FlowControl::None)
        .timeout(Duration::from_millis(10))
        .open()
}

fn virtual_loop(
    mut port: Box<dyn SerialPort>,
    name: String,
    rx: Receiver<Vec<u8>>,
    tx: SyncSender<Command>,
    stop: Arc<AtomicBool>,
    attempt: Arc<AtomicBool>,
    stats: Arc<OutputStats>,
    fault: Arc<Mutex<Option<String>>>,
) {
    let mut buf = [0u8; 4096];
    while !cancelled(&stop, &attempt) {
        match port.read(&mut buf) {
            Ok(n) if n > 0 => {
                let mut command = Command {
                    source: name.clone(),
                    raw: buf[..n].to_vec(),
                };
                loop {
                    if cancelled(&stop, &attempt) {
                        break;
                    }
                    match tx.try_send(command) {
                        Ok(()) => break,
                        Err(TrySendError::Full(c)) => {
                            command = c;
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(TrySendError::Disconnected(_)) => return,
                    }
                }
            }
            Err(e) if !idle(&e) => {
                *fault.lock().unwrap() = Some(format!("虚拟口 {name} 读取失败：{e}"));
                return;
            }
            _ => {}
        }
        // Limit each pass so continuous device traffic cannot starve command reads.
        for _ in 0..128 {
            if cancelled(&stop, &attempt) {
                break;
            }
            let Ok(raw) = rx.try_recv() else {
                break;
            };
            let mut at = 0;
            while at < raw.len() && !cancelled(&stop, &attempt) {
                match port.write(&raw[at..]) {
                    Ok(0) => break,
                    Ok(n) => {
                        at += n;
                        stats.sent.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    Err(e) if idle(&e) => break,
                    Err(e) => {
                        *fault.lock().unwrap() = Some(format!("虚拟口 {name} 写入失败：{e}"));
                        break;
                    }
                }
            }
            stats
                .dropped
                .fetch_add((raw.len() - at) as u64, Ordering::Relaxed);
        }
    }
}

struct SessionIo<'a> {
    group: &'a Group,
    key: u64,
    sink: &'a Sink,
    display: &'a SyncSender<DisplayEvent>,
    stats: &'a Stats,
    clock: Instant,
}
impl SessionIo<'_> {
    fn record(&self, direction: Direction, source: Option<String>, raw: &[u8]) {
        let ms = core::epoch_ms(SystemTime::now());
        let mono_us = self.clock.elapsed().as_micros() as u64;
        if self.group.record {
            self.sink.send(Record::Chunk {
                key: self.key,
                utc: core::utc_iso(ms),
                mono_us,
                direction,
                source: source.clone(),
                raw: raw.to_vec(),
            });
        }
        if self
            .display
            .try_send(DisplayEvent {
                group: self.group.id,
                ms,
                direction,
                source,
                raw: raw.to_vec(),
                notice: None,
            })
            .is_err()
        {
            self.stats.display_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn unconfirmed(&self, source: &str, len: usize) {
        self.stats
            .tx_unconfirmed
            .fetch_add(len as u64, Ordering::Relaxed);
        let text = format!("{source} → 物理口写超时/返回 0：{len} 字节未确认，未自动重发");
        *self.stats.warning.lock().unwrap() = Some(text.clone());
        if self
            .display
            .try_send(DisplayEvent {
                group: self.group.id,
                ms: core::epoch_ms(SystemTime::now()),
                direction: Direction::Tx,
                source: Some(source.into()),
                raw: vec![],
                notice: Some(text),
            })
            .is_err()
        {
            self.stats.display_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn physical_loop(
    port: &mut dyn SerialPort,
    rx: Receiver<Command>,
    outputs: Vec<(SyncSender<Vec<u8>>, Arc<OutputStats>)>,
    stop: &AtomicBool,
    fault: &Mutex<Option<String>>,
    io: &SessionIo<'_>,
) -> Result<(), String> {
    let mut buf = [0u8; 4096];
    while !stop.load(Ordering::Relaxed) {
        if let Some(e) = fault.lock().unwrap().take() {
            return Err(e);
        }
        match port.read(&mut buf) {
            Ok(n) if n > 0 => {
                io.stats.rx.fetch_add(n as u64, Ordering::Relaxed);
                io.record(Direction::Rx, None, &buf[..n]);
                for (tx, stats) in &outputs {
                    if tx.try_send(buf[..n].to_vec()).is_err() {
                        stats.dropped.fetch_add(n as u64, Ordering::Relaxed);
                    }
                }
            }
            Err(e) if !idle(&e) => return Err(format!("物理口读取失败：{e}")),
            _ => {}
        }
        for _ in 0..256 {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let Ok(command) = rx.try_recv() else {
                break;
            };
            port.set_timeout(core::write_timeout(command.raw.len(), io.group.baud))
                .map_err(|e| e.to_string())?;
            let mut at = 0;
            let mut failure = None;
            while at < command.raw.len() && !stop.load(Ordering::Relaxed) {
                match port.write(&command.raw[at..]) {
                    Ok(0) => {
                        io.unconfirmed(&command.source, command.raw.len() - at);
                        break;
                    }
                    Ok(n) => {
                        io.stats.tx.fetch_add(n as u64, Ordering::Relaxed);
                        io.record(
                            Direction::Tx,
                            Some(command.source.clone()),
                            &command.raw[at..at + n],
                        );
                        at += n;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    // A timeout may have written an unknown prefix: never resend it.
                    Err(e) if idle(&e) => {
                        io.unconfirmed(&command.source, command.raw.len() - at);
                        break;
                    }
                    Err(e) => {
                        failure = Some(format!("物理口写入失败（未重发）：{e}"));
                        break;
                    }
                }
            }
            port.set_timeout(Duration::from_millis(10))
                .map_err(|e| e.to_string())?;
            if let Some(e) = failure {
                return Err(e);
            }
        }
    }
    Ok(())
}

impl Runtime {
    pub fn start(
        group: Group,
        pairs: Arc<Mutex<Vec<Pair>>>,
        sink: Sink,
        display: SyncSender<DisplayEvent>,
        clock: Instant,
    ) -> Self {
        let state = Arc::new(Mutex::new(State::Waiting("正在打开串口".into())));
        let stats = Arc::new(Stats {
            rx: AtomicU64::new(0),
            tx: AtomicU64::new(0),
            display_dropped: AtomicU64::new(0),
            tx_unconfirmed: AtomicU64::new(0),
            warning: Mutex::new(None),
            outputs: group
                .outputs
                .iter()
                .map(|n| (n.clone(), Arc::new(OutputStats::default())))
                .collect(),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let exiting = Arc::new(AtomicBool::new(false));
        let (worker_state, worker_stats, worker_stop, worker_exit) =
            (state.clone(), stats.clone(), stop.clone(), exiting.clone());
        let handle = thread::spawn(move || {
            static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
            while !worker_stop.load(Ordering::Relaxed) {
                let attempt = Arc::new(AtomicBool::new(false));
                let fault = Arc::new(Mutex::new(None));
                let (command_tx, command_rx) = mpsc::sync_channel(256);
                let mut outputs = Vec::new();
                let mut handles = Vec::new();
                let result = (|| -> Result<(), String> {
                    let pairs = pairs.lock().unwrap().clone();
                    for (name, (_, counter)) in group.outputs.iter().zip(&worker_stats.outputs) {
                        let pair = pairs
                            .iter()
                            .find(|p| p.a_name.eq_ignore_ascii_case(name))
                            .ok_or_else(|| format!("未检测到虚拟口 {name}，请重新检测后启动"))?;
                        let backend = pair.b_name.clone();
                        let name = name.clone();
                        let (tx, rx) = mpsc::sync_channel(128);
                        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
                        outputs.push((tx, counter.clone()));
                        let (command_tx, stop, attempt, counter, fault) = (
                            command_tx.clone(),
                            worker_stop.clone(),
                            attempt.clone(),
                            counter.clone(),
                            fault.clone(),
                        );
                        handles.push(thread::spawn(move || {
                            let opened = serialport::new(&backend, 115200)
                                .flow_control(FlowControl::None)
                                .timeout(Duration::from_millis(20))
                                .open();
                            match opened {
                                Ok(port) => {
                                    if ready_tx.send(Ok(())).is_ok() {
                                        virtual_loop(
                                            port, name, rx, command_tx, stop, attempt, counter,
                                            fault,
                                        );
                                    }
                                }
                                Err(e) => {
                                    let _ = ready_tx.send(Err(format!("{backend} 打不开：{e}")));
                                }
                            }
                        }));
                        loop {
                            if worker_stop.load(Ordering::Relaxed) {
                                return Ok(());
                            }
                            match ready_rx.recv_timeout(Duration::from_millis(20)) {
                                Ok(result) => {
                                    result?;
                                    break;
                                }
                                Err(mpsc::RecvTimeoutError::Timeout) => {}
                                Err(mpsc::RecvTimeoutError::Disconnected) => {
                                    return Err("虚拟口线程意外退出".into());
                                }
                            }
                        }
                    }
                    if worker_stop.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    let mut physical =
                        open_physical(&group).map_err(|e| format!("{} 打不开：{e}", group.port))?;
                    let key = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
                    if group.record {
                        sink.send(Record::Begin {
                            key,
                            group: group.clone(),
                            utc: core::utc_iso(core::epoch_ms(SystemTime::now())),
                        });
                    }
                    *worker_state.lock().unwrap() = State::Running;
                    let session = SessionIo {
                        group: &group,
                        key,
                        sink: &sink,
                        display: &display,
                        stats: &worker_stats,
                        clock,
                    };
                    let result = physical_loop(
                        physical.as_mut(),
                        command_rx,
                        outputs,
                        &worker_stop,
                        &fault,
                        &session,
                    );
                    drop(physical);
                    let reason = if worker_exit.load(Ordering::Relaxed) {
                        "程序退出".into()
                    } else if worker_stop.load(Ordering::Relaxed) {
                        "用户停止".into()
                    } else {
                        format!(
                            "物理口故障: {}",
                            result
                                .as_ref()
                                .err()
                                .map(String::as_str)
                                .unwrap_or("未知故障")
                        )
                    };
                    if group.record {
                        sink.send(Record::End {
                            key,
                            utc: core::utc_iso(core::epoch_ms(SystemTime::now())),
                            reason,
                        });
                    }
                    result
                })();
                attempt.store(true, Ordering::Relaxed);
                drop(command_tx);
                for h in handles {
                    let _ = h.join();
                }
                if worker_stop.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(e) = result {
                    *worker_state.lock().unwrap() = State::Waiting(e);
                }
                wait_stop(&worker_stop, Duration::from_secs(3));
            }
            *worker_state.lock().unwrap() = State::Stopped;
        });
        Self {
            state,
            stats,
            stop,
            exiting,
            handle: Some(handle),
            stopped: None,
        }
    }
    pub fn stop_async(&mut self) {
        if self.stop.swap(true, Ordering::Relaxed) {
            return;
        }
        *self.state.lock().unwrap() = State::Stopping;
        if let Some(handle) = self.handle.take() {
            let (tx, rx) = mpsc::sync_channel(1);
            self.stopped = Some(rx);
            self.handle = Some(thread::spawn(move || {
                let _ = handle.join();
                let _ = tx.send(());
            }));
        }
    }
    pub fn finished(&mut self) -> bool {
        if self
            .stopped
            .as_ref()
            .is_some_and(|rx| rx.try_recv().is_ok())
        {
            // The worker has already been joined by the reaper. UI only drops its handle.
            self.handle.take();
            true
        } else {
            false
        }
    }
    pub fn shutdown(&mut self) {
        self.exiting.store(true, Ordering::Relaxed);
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recorder::Recorder;
    use std::collections::VecDeque;

    struct FakePort {
        reads: VecDeque<Vec<u8>>,
        writes: Vec<u8>,
        limit: usize,
        fail_after: Option<usize>,
        failure_kind: io::ErrorKind,
        timeouts: Vec<Duration>,
    }
    impl Read for FakePort {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(raw) = self.reads.pop_front() else {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "拔掉设备"));
            };
            buf[..raw.len()].copy_from_slice(&raw);
            Ok(raw.len())
        }
    }
    impl Write for FakePort {
        fn write(&mut self, raw: &[u8]) -> io::Result<usize> {
            if self.fail_after.is_some_and(|n| self.writes.len() >= n) {
                return Err(io::Error::new(self.failure_kind, "写失败"));
            }
            let n = raw.len().min(self.limit);
            self.writes.extend_from_slice(&raw[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl SerialPort for FakePort {
        fn name(&self) -> Option<String> {
            Some("fake".into())
        }
        fn baud_rate(&self) -> serialport::Result<u32> {
            Ok(2400)
        }
        fn data_bits(&self) -> serialport::Result<DataBits> {
            Ok(DataBits::Eight)
        }
        fn flow_control(&self) -> serialport::Result<FlowControl> {
            Ok(FlowControl::None)
        }
        fn parity(&self) -> serialport::Result<serialport::Parity> {
            Ok(serialport::Parity::None)
        }
        fn stop_bits(&self) -> serialport::Result<StopBits> {
            Ok(StopBits::One)
        }
        fn timeout(&self) -> Duration {
            Duration::from_millis(10)
        }
        fn set_baud_rate(&mut self, _: u32) -> serialport::Result<()> {
            Ok(())
        }
        fn set_data_bits(&mut self, _: DataBits) -> serialport::Result<()> {
            Ok(())
        }
        fn set_flow_control(&mut self, _: FlowControl) -> serialport::Result<()> {
            Ok(())
        }
        fn set_parity(&mut self, _: serialport::Parity) -> serialport::Result<()> {
            Ok(())
        }
        fn set_stop_bits(&mut self, _: StopBits) -> serialport::Result<()> {
            Ok(())
        }
        fn set_timeout(&mut self, t: Duration) -> serialport::Result<()> {
            self.timeouts.push(t);
            Ok(())
        }
        fn write_request_to_send(&mut self, _: bool) -> serialport::Result<()> {
            Ok(())
        }
        fn write_data_terminal_ready(&mut self, _: bool) -> serialport::Result<()> {
            Ok(())
        }
        fn read_clear_to_send(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn read_data_set_ready(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn read_ring_indicator(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn read_carrier_detect(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }
        fn bytes_to_read(&self) -> serialport::Result<u32> {
            Ok(0)
        }
        fn bytes_to_write(&self) -> serialport::Result<u32> {
            Ok(0)
        }
        fn clear(&self, _: serialport::ClearBuffer) -> serialport::Result<()> {
            Ok(())
        }
        fn try_clone(&self) -> serialport::Result<Box<dyn SerialPort>> {
            panic!("a serial handle must not be cloned")
        }
        fn set_break(&self) -> serialport::Result<()> {
            Ok(())
        }
        fn clear_break(&self) -> serialport::Result<()> {
            Ok(())
        }
    }
    fn fake(reads: Vec<Vec<u8>>, limit: usize) -> FakePort {
        FakePort {
            reads: reads.into(),
            writes: vec![],
            limit,
            fail_after: None,
            failure_kind: io::ErrorKind::BrokenPipe,
            timeouts: vec![],
        }
    }

    #[test]
    fn physical_fanout_partial_tx_and_fault_recording() {
        static ID: AtomicU64 = AtomicU64::new(1);
        for (fail_after, kind, limit) in [
            (None, io::ErrorKind::BrokenPipe, 2),
            (Some(2), io::ErrorKind::BrokenPipe, 2),
            (Some(2), io::ErrorKind::TimedOut, 2),
            (None, io::ErrorKind::TimedOut, 0),
        ] {
            let path = std::env::current_dir()
                .unwrap()
                .join("target")
                .join(format!(
                    "bridge-test-{}-{}.sqlite3",
                    std::process::id(),
                    ID.fetch_add(1, Ordering::Relaxed)
                ));
            let mut recorder = Recorder::start(&path, 30);
            let mut group = Group::new(1, "test".into());
            group.port = "fake".into();
            recorder.sink.send(Record::Begin {
                key: 1,
                group: group.clone(),
                utc: core::utc_iso(core::epoch_ms(SystemTime::now())),
            });
            let stats = Stats {
                rx: AtomicU64::new(0),
                tx: AtomicU64::new(0),
                display_dropped: AtomicU64::new(0),
                tx_unconfirmed: AtomicU64::new(0),
                warning: Mutex::new(None),
                outputs: vec![],
            };
            let (display, _) = mpsc::sync_channel(1);
            let session = SessionIo {
                group: &group,
                key: 1,
                sink: &recorder.sink,
                display: &display,
                stats: &stats,
                clock: Instant::now(),
            };
            let (commands, rx) = mpsc::sync_channel(1);
            commands
                .send(Command {
                    source: "COM10".into(),
                    raw: vec![1, 2, 3, 4],
                })
                .unwrap();
            let (full, full_rx) = mpsc::sync_channel(1);
            full.send(vec![9]).unwrap();
            let (ready, ready_rx) = mpsc::sync_channel(1);
            let slow = Arc::new(OutputStats::default());
            let fast = Arc::new(OutputStats::default());
            let mut port = fake(vec![vec![0, 255]], limit);
            port.fail_after = fail_after;
            port.failure_kind = kind;
            let result = physical_loop(
                &mut port,
                rx,
                vec![(full, slow.clone()), (ready, fast)],
                &AtomicBool::new(false),
                &Mutex::new(None),
                &session,
            );
            assert!(result.is_err());
            assert_eq!(ready_rx.try_recv().unwrap(), vec![0, 255]);
            assert_eq!(full_rx.try_recv().unwrap(), vec![9]);
            assert_eq!(slow.dropped.load(Ordering::Relaxed), 2);
            assert_eq!(stats.rx.load(Ordering::Relaxed), 2);
            let expected = if limit == 0 {
                vec![]
            } else if fail_after.is_some() {
                vec![1, 2]
            } else {
                vec![1, 2, 3, 4]
            };
            if kind == io::ErrorKind::TimedOut {
                assert_eq!(
                    stats.tx_unconfirmed.load(Ordering::Relaxed),
                    4 - expected.len() as u64
                );
                assert!(
                    stats
                        .warning
                        .lock()
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .contains("未自动重发")
                );
                assert!(result.unwrap_err().contains("读取失败"));
            }
            assert_eq!(port.writes, expected);
            assert_eq!(
                port.timeouts,
                [core::write_timeout(4, 2400), Duration::from_millis(10)]
            );
            assert!(stats.display_dropped.load(Ordering::Relaxed) > 0);
            recorder.sink.send(Record::End {
                key: 1,
                utc: core::utc_iso(core::epoch_ms(SystemTime::now())),
                reason: "物理口故障: test".into(),
            });
            recorder.shutdown();
            assert!(recorder.sink.error.lock().unwrap().is_none());
            let c = rusqlite::Connection::open(&path).unwrap();
            let mut query = c
                .prepare("SELECT raw,source FROM chunks WHERE direction='TX' ORDER BY id")
                .unwrap();
            let rows = query
                .query_map([], |r| {
                    Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(rows.iter().all(|(_, source)| source == "COM10"));
            assert_eq!(
                rows.into_iter()
                    .flat_map(|(raw, _)| raw)
                    .collect::<Vec<_>>(),
                expected
            );
            drop(query);
            drop(c);
            for file in [
                path.clone(),
                path.with_extension("sqlite3-wal"),
                path.with_extension("sqlite3-shm"),
            ] {
                let _ = std::fs::remove_file(file);
            }
        }
    }

    #[test]
    fn virtual_zero_write_is_a_counted_drop() {
        let (out, rx) = mpsc::sync_channel(1);
        out.send(vec![1, 2, 3]).unwrap();
        let (commands, _) = mpsc::sync_channel(1);
        let stats = Arc::new(OutputStats::default());
        virtual_loop(
            Box::new(fake(vec![vec![]], 0)),
            "COM10".into(),
            rx,
            commands,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            stats.clone(),
            Arc::new(Mutex::new(None)),
        );
        assert_eq!(stats.sent.load(Ordering::Relaxed), 0);
        assert_eq!(stats.dropped.load(Ordering::Relaxed), 3);
    }
    #[test]
    fn command_backpressure_waits_and_can_be_stopped() {
        let (commands, rx) = mpsc::sync_channel(1);
        commands
            .send(Command {
                source: "first".into(),
                raw: vec![9],
            })
            .unwrap();
        let (_, outputs) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let h = thread::spawn(move || {
            virtual_loop(
                Box::new(fake(vec![vec![1, 2]], 2)),
                "COM10".into(),
                outputs,
                commands,
                worker_stop,
                Arc::new(AtomicBool::new(false)),
                Arc::new(OutputStats::default()),
                Arc::new(Mutex::new(None)),
            )
        });
        thread::sleep(Duration::from_millis(30));
        assert!(!h.is_finished());
        stop.store(true, Ordering::Relaxed);
        h.join().unwrap();
        assert_eq!(rx.try_recv().unwrap().raw, vec![9]);
        assert!(rx.try_recv().is_err());
    }
}
