//! Run with the bridge active: cargo run --example loopback -- CNCB2 COM10 COM11
use serialport::SerialPort;
use std::time::{Duration, Instant};

fn read_until(port: &mut dyn SerialPort, expected: usize) -> Result<Vec<u8>, String> {
    let start = Instant::now();
    let mut result = Vec::new();
    let mut buf = [0u8; 4096];
    while result.len() < expected && start.elapsed() < Duration::from_secs(10) {
        match port.read(&mut buf) {
            Ok(n) => result.extend_from_slice(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(result)
}
fn open(name: &str) -> Result<Box<dyn SerialPort>, String> {
    serialport::new(name, 115200)
        .timeout(Duration::from_millis(20))
        .flow_control(serialport::FlowControl::None)
        .open()
        .map_err(|e| format!("{name}: {e}"))
}
fn run() -> Result<bool, String> {
    let names = std::env::args().skip(1).collect::<Vec<_>>();
    if names.len() < 2 {
        return Err("用法：loopback <假物理口的B端> <虚拟口A端...>".into());
    }
    let mut physical = open(&names[0])?;
    let mut outputs = names[1..]
        .iter()
        .map(|n| open(n))
        .collect::<Result<Vec<_>, _>>()?;
    physical
        .clear(serialport::ClearBuffer::All)
        .map_err(|e| e.to_string())?;
    for p in &outputs {
        p.clear(serialport::ClearBuffer::All)
            .map_err(|e| e.to_string())?;
    }
    let mut passed = true;
    for sequence in 1..=5 {
        let raw = format!("[RX-{sequence:04}] serial-split \0\u{7f}\r\n").into_bytes();
        physical.write_all(&raw).map_err(|e| e.to_string())?;
        for (name, port) in names[1..].iter().zip(&mut outputs) {
            let received = read_until(port.as_mut(), raw.len())?;
            let ok = received == raw;
            passed &= ok;
            println!(
                "{} RX #{sequence}: {} → {name} ({} / {} bytes)",
                if ok { "PASS" } else { "FAIL" },
                names[0],
                received.len(),
                raw.len()
            );
        }
    }
    // Issue from all clients before reading the device side, so interleaving is exercised.
    let commands = names[1..]
        .iter()
        .enumerate()
        .map(|(i, n)| format!("[TX-{i:04}<{n}>] command-{i:04}\r\n").into_bytes())
        .collect::<Vec<_>>();
    for (port, raw) in outputs.iter_mut().zip(&commands) {
        port.write_all(raw).map_err(|e| e.to_string())?;
    }
    let expected = commands.iter().map(Vec::len).sum();
    let mut received = read_until(physical.as_mut(), expected)?;
    for (name, raw) in names[1..].iter().zip(&commands) {
        let position = received.windows(raw.len()).position(|window| window == raw);
        let ok = position.is_some();
        passed &= ok;
        println!(
            "{} TX: {name} → {} (完整 {} bytes)",
            if ok { "PASS" } else { "FAIL" },
            names[0],
            raw.len()
        );
        if let Some(at) = position {
            received.drain(at..at + raw.len());
        }
    }
    let ok = received.is_empty();
    passed &= ok;
    println!(
        "{} TX: 无多余数据 ({} bytes)",
        if ok { "PASS" } else { "FAIL" },
        received.len()
    );
    Ok(passed)
}
fn main() {
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("FAIL: {e}");
            std::process::exit(1);
        }
    }
}
