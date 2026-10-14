//! 纯规则：分组配置的读写、校验、时间与显示格式。不碰串口、文件、注册表和界面。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const BAUDS: [u32; 12] = [
    300, 600, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800,
];
pub const DEFAULT_BAUD: u32 = 2400;
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

impl Parity {
    pub const ALL: [Parity; 3] = [Parity::None, Parity::Even, Parity::Odd];
    pub fn key(self) -> &'static str {
        match self {
            Parity::None => "none",
            Parity::Even => "even",
            Parity::Odd => "odd",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Parity::None => "无",
            Parity::Even => "偶",
            Parity::Odd => "奇",
        }
    }
    pub fn short(self) -> char {
        match self {
            Parity::None => 'N',
            Parity::Even => 'E',
            Parity::Odd => 'O',
        }
    }
    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    Ask,
    Tray,
    Exit,
}

impl CloseAction {
    pub const ALL: [CloseAction; 3] = [CloseAction::Ask, CloseAction::Tray, CloseAction::Exit];
    pub fn key(self) -> &'static str {
        match self {
            CloseAction::Ask => "ask",
            CloseAction::Tray => "tray",
            CloseAction::Exit => "exit",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            CloseAction::Ask => "每次询问",
            CloseAction::Tray => "最小化到托盘",
            CloseAction::Exit => "退出程序",
        }
    }
    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.key() == s)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub id: u32,
    pub name: String,
    pub port: String,
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
    /// 选中的虚拟口，记的是给其他软件用的那一端的名字（如 COM10）。
    pub outputs: Vec<String>,
    pub record: bool,
    /// 上次退出时是否在运行；启动后据此自动恢复。
    pub running: bool,
}

impl Group {
    pub fn new(id: u32, name: String) -> Self {
        Group {
            id,
            name,
            port: String::new(),
            baud: DEFAULT_BAUD,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
            outputs: Vec::new(),
            record: true,
            running: false,
        }
    }

    pub fn line_settings(&self) -> String {
        format!(
            "{} {}{}{}",
            self.baud,
            self.data_bits,
            self.parity.short(),
            self.stop_bits
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}
impl Theme {
    pub fn key(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
    pub fn from_key(value: &str) -> Option<Self> {
        match value {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: Theme,
    pub show_data: bool,
    pub close_action: CloseAction,
    pub retention_days: u32,
    pub groups: Vec<Group>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::Light,
            show_data: false,
            close_action: CloseAction::Ask,
            retention_days: DEFAULT_RETENTION_DAYS,
            groups: Vec::new(),
        }
    }
}

impl Settings {
    pub fn next_group_id(&self) -> u32 {
        self.groups.iter().map(|g| g.id).max().unwrap_or(0) + 1
    }

    pub fn next_group_name(&self) -> String {
        (1..)
            .map(|n| format!("分组{n}"))
            .find(|name| !self.groups.iter().any(|g| &g.name == name))
            .unwrap()
    }

    pub fn group(&self, id: u32) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_mut(&mut self, id: u32) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    /// 虚拟口被哪个分组占用（排除 `except` 自己）。
    pub fn output_owner(&self, output: &str, except: u32) -> Option<&Group> {
        self.groups
            .iter()
            .find(|g| g.id != except && g.outputs.iter().any(|o| o.eq_ignore_ascii_case(output)))
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

/// 读 settings.ini。格式：全局键值在前，每个分组一个 `[group.<id>]` 段。坏值保留默认值。
pub fn parse_settings(text: &str) -> Settings {
    let mut s = Settings::default();
    let mut current: Option<usize> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = section
                .strip_prefix("group.")
                .and_then(|id| id.parse::<u32>().ok())
                .filter(|id| *id > 0 && s.group(*id).is_none())
                .map(|id| {
                    s.groups.push(Group::new(id, format!("分组{id}")));
                    s.groups.len() - 1
                });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match current {
            None => match key {
                "theme" => {
                    if let Some(theme) = Theme::from_key(value) {
                        s.theme = theme;
                    }
                }
                "show_data" => {
                    if let Some(show) = parse_bool(value) {
                        s.show_data = show;
                    }
                }
                "close_action" => {
                    if let Some(c) = CloseAction::from_key(value) {
                        s.close_action = c;
                    }
                }
                "retention_days" => {
                    if let Ok(d) = value.parse::<u32>() {
                        if d > 0 {
                            s.retention_days = d;
                        }
                    }
                }
                _ => {}
            },
            Some(i) => {
                let g = &mut s.groups[i];
                match key {
                    "name" if !value.is_empty() => g.name = value.to_string(),
                    "port" => g.port = value.to_string(),
                    "baud" => {
                        if let Ok(b) = value.parse::<u32>() {
                            if b > 0 {
                                g.baud = b;
                            }
                        }
                    }
                    "data_bits" => {
                        if let Ok(b @ (5..=8)) = value.parse::<u8>() {
                            g.data_bits = b;
                        }
                    }
                    "parity" => {
                        if let Some(p) = Parity::from_key(value) {
                            g.parity = p;
                        }
                    }
                    "stop_bits" => {
                        if let Ok(b @ (1..=2)) = value.parse::<u8>() {
                            g.stop_bits = b;
                        }
                    }
                    "outputs" => {
                        g.outputs = value
                            .split(',')
                            .map(str::trim)
                            .filter(|o| !o.is_empty())
                            .map(str::to_string)
                            .collect();
                    }
                    "record" => {
                        if let Some(b) = parse_bool(value) {
                            g.record = b;
                        }
                    }
                    "running" => {
                        if let Some(b) = parse_bool(value) {
                            g.running = b;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    s
}

pub fn format_settings(s: &Settings) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "theme={}\nshow_data={}\n",
        s.theme.key(),
        s.show_data
    ));
    out.push_str(&format!("close_action={}\n", s.close_action.key()));
    out.push_str(&format!("retention_days={}\n", s.retention_days));
    for g in &s.groups {
        out.push_str(&format!("\n[group.{}]\n", g.id));
        out.push_str(&format!("name={}\n", g.name.replace(['\r', '\n'], " ")));
        out.push_str(&format!("port={}\n", g.port));
        out.push_str(&format!("baud={}\n", g.baud));
        out.push_str(&format!("data_bits={}\n", g.data_bits));
        out.push_str(&format!("parity={}\n", g.parity.key()));
        out.push_str(&format!("stop_bits={}\n", g.stop_bits));
        out.push_str(&format!("outputs={}\n", g.outputs.join(",")));
        out.push_str(&format!("record={}\n", g.record));
        out.push_str(&format!("running={}\n", g.running));
    }
    out
}

/// 启动一个分组前的检查。`running_ports` 是其他正在运行的分组占着的物理口。
pub fn validate_start(g: &Group, running_ports: &[String]) -> Result<(), String> {
    if g.port.is_empty() {
        return Err("还没选物理口".into());
    }
    if g.outputs.is_empty() {
        return Err("至少勾选一个虚拟口".into());
    }
    if g.outputs.iter().any(|o| o.eq_ignore_ascii_case(&g.port)) {
        return Err(format!("{} 既是物理口又是虚拟口", g.port));
    }
    if running_ports
        .iter()
        .any(|p| p.eq_ignore_ascii_case(&g.port))
    {
        return Err(format!("{} 已被另一个运行中的分组占用", g.port));
    }
    Ok(())
}

/// COM 口自然排序：COM2 排在 COM10 前面。
pub fn port_sort_key(name: &str) -> (String, u32, String) {
    let digits_at = name
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(name.len());
    let (prefix, rest) = name.split_at(digits_at);
    let num_len = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let num = rest[..num_len].parse::<u32>().unwrap_or(u32::MAX);
    (
        prefix.to_ascii_uppercase(),
        num,
        rest[num_len..].to_string(),
    )
}

/// 物理口写超时：按最慢 11 位/字节估算传输时间，翻倍再加 500ms 余量。
pub fn write_timeout(len: usize, baud: u32) -> Duration {
    let ms = (len as u64 * 11 * 1000 * 2) / baud.max(1) as u64 + 500;
    Duration::from_millis(ms)
}

/// 1970-01-01 起的天数 → (年, 月, 日)。Howard Hinnant 的 civil_from_days。
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn split_epoch_ms(ms: i64) -> (i64, u32, u32, u32, u32) {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let h = (rem / 3_600_000) as u32;
    let m = (rem / 60_000 % 60) as u32;
    let s = (rem / 1000 % 60) as u32;
    let milli = (rem % 1000) as u32;
    (days, h, m, s, milli)
}

pub fn epoch_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// `2026-10-14T03:04:05.678Z`
pub fn utc_iso(ms: i64) -> String {
    let (days, h, m, s, milli) = split_epoch_ms(ms);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.{milli:03}Z")
}

/// 本地时间 `HH:MM:SS.mmm`，`offset_minutes` 为本地相对 UTC 的偏移（东八区 +480）。
pub fn local_hms(ms: i64, offset_minutes: i32) -> String {
    let (_, h, m, s, milli) = split_epoch_ms(ms + offset_minutes as i64 * 60_000);
    format!("{h:02}:{m:02}:{s:02}.{milli:03}")
}

pub fn retention_cutoff(now_ms: i64, days: u32) -> String {
    utc_iso(now_ms - days as i64 * 86_400_000)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// 设备 → 物理口
    Rx,
    /// 虚拟口上的软件 → 物理口 → 设备
    Tx,
}

impl Direction {
    pub fn key(self) -> &'static str {
        match self {
            Direction::Rx => "RX",
            Direction::Tx => "TX",
        }
    }
}

pub const DISPLAY_HEX_LIMIT: usize = 64;

/// 实时数据区的一行：时间、方向（TX 带来源虚拟口）、字节数、HEX、可打印字符。
pub fn display_line(hms: &str, dir: Direction, source: Option<&str>, raw: &[u8]) -> String {
    let shown = &raw[..raw.len().min(DISPLAY_HEX_LIMIT)];
    let hex: Vec<String> = shown.iter().map(|b| format!("{b:02X}")).collect();
    let ascii: String = shown
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            // The existing display contract renders a leading 0x68 frame marker as '.'.
            if (0x20..0x7f).contains(&b) && !(i == 0 && b == 0x68) {
                b as char
            } else {
                '.'
            }
        })
        .collect();
    let more = if raw.len() > shown.len() {
        format!(" …(+{})", raw.len() - shown.len())
    } else {
        String::new()
    };
    let tag = match (dir, source) {
        (Direction::Tx, Some(src)) => format!("TX←{src}"),
        _ => dir.key().to_string(),
    };
    format!(
        "{hms}  {tag:<8} {:>5}B  {}{more}  |{ascii}|",
        raw.len(),
        hex.join(" ")
    )
}

pub fn display_text_line(hms: &str, dir: Direction, source: Option<&str>, raw: &[u8]) -> String {
    let text: String = String::from_utf8_lossy(raw)
        .chars()
        .map(|c| if c.is_control() { '.' } else { c })
        .collect();
    let tag = match (dir, source) {
        (Direction::Tx, Some(source)) => format!("TX←{source}"),
        _ => dir.key().into(),
    };
    format!("[{hms}] {tag} {text}")
}

pub fn display_hex_line(hms: &str, dir: Direction, source: Option<&str>, raw: &[u8]) -> String {
    let shown = &raw[..raw.len().min(DISPLAY_HEX_LIMIT)];
    let hex = shown
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    let tag = match (dir, source) {
        (Direction::Tx, Some(source)) => format!("TX←{source}"),
        _ => dir.key().into(),
    };
    let more = if shown.len() < raw.len() {
        format!(" …(+{})", raw.len() - shown.len())
    } else {
        String::new()
    };
    format!("[{hms}] {tag} {hex}{more}")
}

pub fn human_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / 1024.0 / 1024.0)
    }
}

/// 建虚拟口对时推荐的下一个名字：COM10 起，跳过已存在的。
pub fn suggest_port_name(existing: &[String]) -> String {
    (10u32..=255)
        .map(|n| format!("COM{n}"))
        .find(|name| !existing.iter().any(|e| e.eq_ignore_ascii_case(name)))
        .unwrap_or_else(|| "COM#".into())
}

/// 用户输入的虚拟口名：COM 加 1-255 的编号，或 COM# 表示让驱动自动编号。
pub fn normalize_port_name(input: &str) -> Result<String, String> {
    let s = input.trim().to_ascii_uppercase();
    if s == "COM#" {
        return Ok(s);
    }
    match s.strip_prefix("COM").and_then(|n| n.parse::<u32>().ok()) {
        Some(n @ 1..=255) => Ok(format!("COM{n}")),
        _ => Err(format!(
            "“{}”不是有效的串口名，要写成 COM13 这样",
            input.trim()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Settings {
        let mut g1 = Group::new(1, "红外口".into());
        g1.port = "COM6".into();
        g1.parity = Parity::Even;
        g1.outputs = vec!["COM10".into(), "COM11".into()];
        g1.running = true;
        let mut g2 = Group::new(3, "调试口".into());
        g2.baud = 115200;
        g2.record = false;
        Settings {
            close_action: CloseAction::Tray,
            retention_days: 7,
            groups: vec![g1, g2],
            ..Settings::default()
        }
    }

    #[test]
    fn settings_roundtrip() {
        let s = sample();
        assert_eq!(parse_settings(&format_settings(&s)), s);
    }

    #[test]
    fn settings_bad_values_keep_defaults() {
        let s = parse_settings(
            "close_action=bogus\nretention_days=0\n[group.2]\nbaud=abc\nparity=x\nstop_bits=9\n[group.0]\nname=丢弃\n[group.2]\nname=重复\n",
        );
        assert_eq!(s.close_action, CloseAction::Ask);
        assert_eq!(s.retention_days, DEFAULT_RETENTION_DAYS);
        assert_eq!(s.groups.len(), 1);
        let g = &s.groups[0];
        assert_eq!(
            (g.baud, g.parity, g.stop_bits),
            (DEFAULT_BAUD, Parity::None, 1)
        );
        assert_eq!(g.name, "分组2");
    }

    #[test]
    fn next_ids_and_names() {
        let s = sample();
        assert_eq!(s.next_group_id(), 4);
        assert_eq!(s.next_group_name(), "分组1");
        assert_eq!(s.output_owner("COM11", 3).map(|g| g.id), Some(1));
        assert!(s.output_owner("COM11", 1).is_none());
    }

    #[test]
    fn validate_start_rules() {
        let s = sample();
        let g = &s.groups[0];
        assert!(validate_start(g, &[]).is_ok());
        assert!(validate_start(g, &["com6".into()]).is_err());
        let mut bad = g.clone();
        bad.outputs.clear();
        assert!(validate_start(&bad, &[]).is_err());
        bad.outputs = vec!["COM6".into()];
        assert!(validate_start(&bad, &[]).is_err());
        assert!(validate_start(&s.groups[1], &[]).is_err());
    }

    #[test]
    fn port_order_is_natural() {
        let mut v = vec!["COM10", "COM2", "CNCB0", "COM1"];
        v.sort_by_key(|p| port_sort_key(p));
        assert_eq!(v, ["CNCB0", "COM1", "COM2", "COM10"]);
    }

    #[test]
    fn time_formatting() {
        assert_eq!(utc_iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(utc_iso(1_791_947_045_678), "2026-10-14T03:04:05.678Z");
        assert_eq!(local_hms(1_791_947_045_678, 480), "11:04:05.678");
        assert_eq!(local_hms(1_791_947_045_678, -240), "23:04:05.678");
        assert_eq!(
            retention_cutoff(86_400_000 * 31, 30),
            "1970-01-02T00:00:00.000Z"
        );
    }

    #[test]
    fn display_line_format_and_truncation() {
        let l = display_line("11:04:05.678", Direction::Tx, Some("COM10"), b"\x68A\x16");
        assert_eq!(l, "11:04:05.678  TX←COM10     3B  68 41 16  |.A.|");
        let long = vec![0x41u8; 70];
        let l = display_line("t", Direction::Rx, None, &long);
        assert!(l.contains("70B"));
        assert!(l.contains("…(+6)"));
    }

    #[test]
    fn write_timeout_scales_with_baud() {
        assert_eq!(write_timeout(0, 2400), Duration::from_millis(500));
        assert_eq!(write_timeout(240, 2400), Duration::from_millis(2700));
        assert!(write_timeout(240, 115200) < Duration::from_millis(600));
    }

    #[test]
    fn port_names() {
        assert_eq!(
            suggest_port_name(&["COM10".into(), "com11".into()]),
            "COM12"
        );
        assert_eq!(normalize_port_name(" com13 ").unwrap(), "COM13");
        assert_eq!(normalize_port_name("com#").unwrap(), "COM#");
        assert!(normalize_port_name("COM0").is_err());
        assert!(normalize_port_name("COM256").is_err());
        assert!(normalize_port_name("CNCA0").is_err());
    }

    #[test]
    fn output_ownership_uses_windows_case_insensitive_names() {
        let settings = sample();
        assert_eq!(settings.output_owner("com11", 3).map(|g| g.id), Some(1));
        assert!(settings.output_owner("com11", 1).is_none());
    }
    #[test]
    fn ui_settings_old_bad_and_new_values() {
        let old = parse_settings("close_action=tray\n[group.1]\nname=旧分组\n");
        assert_eq!(old.theme, Theme::Light);
        assert!(!old.show_data);
        let bad = parse_settings("theme=unknown\nshow_data=maybe\n");
        assert_eq!(bad.theme, Theme::Light);
        assert!(!bad.show_data);
        let mut new = sample();
        new.theme = Theme::Dark;
        new.show_data = true;
        assert_eq!(parse_settings(&format_settings(&new)), new);
    }
    #[test]
    fn new_display_modes_preserve_text_and_replace_controls() {
        assert_eq!(
            display_text_line(
                "12:34:56.789",
                Direction::Tx,
                Some("COM11"),
                b"AT\r\n\0\t\xff"
            ),
            "[12:34:56.789] TX←COM11 AT....�"
        );
        assert_eq!(
            display_text_line("t", Direction::Rx, None, "中文🙂\n".as_bytes()),
            "[t] RX 中文🙂."
        );
        assert_eq!(
            display_hex_line("t", Direction::Rx, None, &[0, 0x68, 255]),
            "[t] RX 00 68 FF"
        );
        assert!(display_hex_line("t", Direction::Rx, None, &[1; 70]).ends_with("…(+6)"));
    }
}
