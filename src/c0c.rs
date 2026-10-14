use crate::{core, winsys::wide};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_CANCELLED},
    System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
    UI::{
        Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW, ShellExecuteW},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
};
use winreg::{RegKey, enums::*};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pair {
    pub index: u32,
    pub a_name: String,
    pub b_name: String,
    pub b_hidden: bool,
}

#[derive(Clone, Debug)]
pub enum HiddenMode {
    Text(String),
    Dword(u32),
}
impl HiddenMode {
    fn enabled(&self) -> bool {
        match self {
            Self::Text(value) => value.eq_ignore_ascii_case("yes"),
            Self::Dword(value) => *value != 0,
        }
    }
}

fn parameter_name(name: &str) -> Option<(u32, bool)> {
    let name = name.to_ascii_uppercase();
    if let Some(n) = name.strip_prefix("CNCA") {
        n.parse().ok().map(|i| (i, true))
    } else {
        name.strip_prefix("CNCB")
            .and_then(|n| n.parse().ok())
            .map(|i| (i, false))
    }
}
fn device_name(name: &str) -> Option<(u32, bool)> {
    let lower = name.to_ascii_lowercase();
    let tail = lower.strip_prefix("\\device\\com0com")?;
    let (side, index) = tail.split_at_checked(1)?;
    match side {
        "1" => index.parse().ok().map(|i| (i, true)),
        "2" => index.parse().ok().map(|i| (i, false)),
        _ => None,
    }
}
fn merge(
    map: &mut BTreeMap<u32, (Option<String>, Option<String>)>,
    index: u32,
    a: bool,
    name: String,
) {
    let item = map.entry(index).or_default();
    let slot = if a { &mut item.0 } else { &mut item.1 };
    if slot.is_none() {
        *slot = Some(name);
    }
}

pub fn parse_pairs(
    parameters: &[(String, Option<String>, Option<HiddenMode>)],
    serial_comm: &[(String, String)],
) -> Vec<Pair> {
    let mut map = BTreeMap::new();
    let mut hidden = BTreeMap::new();
    for (key, value, mode) in parameters {
        if let Some((index, a)) = parameter_name(key) {
            if !a {
                hidden.insert(index, mode.as_ref().is_some_and(HiddenMode::enabled));
            }
            let name = value
                .as_ref()
                .filter(|name| !name.is_empty())
                .unwrap_or(key)
                .clone();
            merge(&mut map, index, a, name);
        }
    }
    for (device, port) in serial_comm {
        if let Some((index, a)) = device_name(device) {
            if !port.is_empty() {
                merge(&mut map, index, a, port.clone());
            }
        }
    }
    map.into_iter()
        .filter_map(|(index, (a, b))| {
            Some(Pair {
                index,
                a_name: a?,
                b_name: b.unwrap_or_else(|| format!("CNCB{index}")),
                b_hidden: hidden.get(&index).copied().unwrap_or(false),
            })
        })
        .collect()
}

/// Only the registry snapshot reader depends on Windows; parsing is independently testable.
pub fn detect_pairs() -> Vec<Pair> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let mut values = Vec::new();
    let mut devices = Vec::new();
    if let Ok(parameters) =
        hklm.open_subkey("SYSTEM\\CurrentControlSet\\Services\\com0com\\Parameters")
    {
        for key in parameters.enum_keys().filter_map(Result::ok) {
            let port = parameters.open_subkey(&key).ok();
            let name = port
                .as_ref()
                .and_then(|k| k.get_value::<String, _>("PortName").ok());
            let mode = port.as_ref().and_then(|k| {
                k.get_value::<String, _>("HiddenMode")
                    .ok()
                    .map(HiddenMode::Text)
                    .or_else(|| {
                        k.get_value::<u32, _>("HiddenMode")
                            .ok()
                            .map(HiddenMode::Dword)
                    })
            });
            values.push((key, name, mode));
        }
    }
    if let Ok(comm) = hklm.open_subkey("HARDWARE\\DEVICEMAP\\SERIALCOMM") {
        for (name, _) in comm.enum_values().filter_map(Result::ok) {
            if let Ok(port) = comm.get_value::<String, _>(&name) {
                devices.push((name, port));
            }
        }
    }
    parse_pairs(&values, &devices)
}

#[derive(Clone)]
pub struct Port {
    pub name: String,
    pub label: String,
}
pub fn physical_ports(pairs: &[Pair], include_virtual: bool) -> Result<Vec<Port>, String> {
    Ok(filter_physical_ports(
        serialport::available_ports().map_err(|e| e.to_string())?,
        pairs,
        include_virtual,
    ))
}

fn filter_physical_ports(
    available: Vec<serialport::SerialPortInfo>,
    pairs: &[Pair],
    include_virtual: bool,
) -> Vec<Port> {
    let mut ports: Vec<Port> = available
        .into_iter()
        .filter_map(|p| {
            let a = pairs
                .iter()
                .any(|pair| pair.a_name.eq_ignore_ascii_case(&p.port_name));
            let b = pairs
                .iter()
                .any(|pair| pair.b_name.eq_ignore_ascii_case(&p.port_name));
            if b || a && !include_virtual {
                return None;
            }
            let suffix = if a {
                "（虚拟）"
            } else if matches!(p.port_type, serialport::SerialPortType::BluetoothPort) {
                "（蓝牙）"
            } else {
                ""
            };
            Some(Port {
                label: port_label(&p, suffix),
                name: p.port_name,
            })
        })
        .collect();
    if include_virtual {
        for pair in pairs {
            if !ports
                .iter()
                .any(|p| p.name.eq_ignore_ascii_case(&pair.a_name))
            {
                ports.push(Port {
                    name: pair.a_name.clone(),
                    label: format!("{}（虚拟）", pair.a_name),
                });
            }
        }
    }
    ports.sort_by_key(|p| core::port_sort_key(&p.name));
    ports
}
fn port_label(port: &serialport::SerialPortInfo, suffix: &str) -> String {
    let description = match &port.port_type {
        serialport::SerialPortType::UsbPort(usb) => usb
            .product
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .or(usb.manufacturer.as_deref()),
        _ => None,
    }
    .filter(|s| !s.trim().is_empty());
    format!(
        "{}{}{suffix}",
        port.port_name,
        description.map(|s| format!(" {s}")).unwrap_or_default()
    )
}
pub fn setup_path() -> Option<PathBuf> {
    ["ProgramFiles(x86)", "ProgramFiles"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|p| PathBuf::from(p).join("com0com/setupc.exe"))
        .find(|p| p.is_file())
}
pub enum Manage {
    Create(String),
    Delete(u32),
    HideBackends(Vec<u32>),
}
fn create_args(name: &str) -> Result<String, String> {
    Ok(format!(
        "--silent install PortName={} HiddenMode=yes",
        core::normalize_port_name(name)?
    ))
}
fn setup_commands(action: &Manage) -> Result<Vec<String>, String> {
    Ok(match action {
        Manage::Create(name) => vec![create_args(name)?],
        Manage::Delete(index) => vec![format!("--silent remove {index}")],
        Manage::HideBackends(indices) => {
            let mut indices = indices.clone();
            indices.sort_unstable();
            indices.dedup();
            indices
                .into_iter()
                .map(|index| format!("--silent change CNCB{index} HiddenMode=yes"))
                .collect()
        }
    })
}
fn batch_path(path: &Path) -> String {
    path.to_string_lossy().replace('%', "%%")
}
fn setup_script(setup: &Path, commands: &[String], log: &Path) -> Result<String, String> {
    if !setup.is_absolute() || !log.is_absolute() {
        return Err("setupc 和日志路径必须是绝对路径".into());
    }
    let directory = batch_path(setup.parent().ok_or("setupc 目录无效")?);
    let setup = batch_path(setup);
    let log = batch_path(log);
    let mut script = format!(
        "cd /d \"{directory}\" >> \"{log}\" 2>&1\r\n@if errorlevel 1 (\r\n@echo [%date% %time%] cd /d \"{directory}\" failed >> \"{log}\"\r\n@exit /b 1\r\n)\r\n@echo off\r\nsetlocal EnableExtensions DisableDelayedExpansion\r\nset \"serial_split_failed=0\"\r\n"
    );
    for command in commands {
        script.push_str(&format!(
            "echo [%date% %time%] \"{setup}\" {command} >> \"{log}\"\r\n\"{setup}\" {command} >> \"{log}\" 2>&1\r\nif errorlevel 1 set \"serial_split_failed=1\"\r\n"
        ));
    }
    script.push_str("exit /b %serial_split_failed%\r\n");
    Ok(script)
}
fn script_args(script: &Path) -> String {
    // The script is UTF-8, including any non-ASCII setupc or user TEMP paths.
    format!(
        "/d /s /v:off /c \"chcp 65001 >nul && \"{}\"\"",
        script.display()
    )
}
fn setup_error(error: String, log: &Path) -> String {
    if error == "已取消" {
        error
    } else {
        format!("{error}\n日志：{}", log.display())
    }
}
struct TemporaryScript(PathBuf);
impl Drop for TemporaryScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub fn manage(action: Manage) -> Result<(), String> {
    let setup = setup_path().ok_or("没装 com0com")?;
    let commands = setup_commands(&action)?;
    if commands.is_empty() {
        return Ok(());
    }
    run_setup(&setup, &commands)
}
fn run_setup(setup: &Path, commands: &[String]) -> Result<(), String> {
    let temp = std::env::temp_dir();
    let log = temp.join("serial-split-setupc.log");
    let result = (|| {
        let setup = std::path::absolute(setup).map_err(|e| e.to_string())?;
        let text = setup_script(&setup, commands, &log)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let path = temp.join(format!(
            "serial-split-setupc-{}-{stamp}.cmd",
            std::process::id()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("无法创建 setupc 命令：{e}"))?;
        let script = TemporaryScript(path);
        let written = std::io::Write::write_all(&mut file, text.as_bytes());
        drop(file);
        written.map_err(|e| format!("无法写入 setupc 命令：{e}"))?;
        let cmd = PathBuf::from(std::env::var_os("SystemRoot").ok_or("未找到 SystemRoot")?)
            .join("System32")
            .join("cmd.exe");
        elevated(&cmd, &script_args(&script.0), setup.parent().unwrap())
    })();
    result.map_err(|error| setup_error(error, &log))
}
fn elevated(file: &std::path::Path, args: &str, directory: &std::path::Path) -> Result<(), String> {
    let (verb, file, params, dir) = (wide("runas"), wide(file), wide(args), wide(directory));
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.lpDirectory = dir.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        if ShellExecuteExW(&mut info) == 0 {
            let e = std::io::Error::last_os_error();
            return Err(if e.raw_os_error() == Some(ERROR_CANCELLED as i32) {
                "已取消".into()
            } else {
                format!("setupc 启动失败：{e}")
            });
        }
        if info.hProcess.is_null() {
            return Err("setupc 未返回进程句柄".into());
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 0;
        let ok = GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        if ok == 0 {
            Err(std::io::Error::last_os_error().to_string())
        } else if code != 0 {
            Err(format!("setupc 返回 {code}"))
        } else {
            Ok(())
        }
    }
}
pub fn open_directory(path: &std::path::Path) -> Result<(), String> {
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            wide("open").as_ptr(),
            wide(path).as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err(format!("无法打开数据目录：{}", result as isize))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usb_description_is_display_only_and_keeps_suffix() {
        let mut port = serialport::SerialPortInfo {
            port_name: "COM6".into(),
            port_type: serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
                vid: 0x1a86,
                pid: 0x7523,
                serial_number: None,
                manufacturer: Some("WCH".into()),
                product: Some("USB-SERIAL CH340".into()),
            }),
        };
        assert_eq!(
            port_label(&port, "（虚拟）"),
            "COM6 USB-SERIAL CH340（虚拟）"
        );
        if let serialport::SerialPortType::UsbPort(usb) = &mut port.port_type {
            usb.product = None;
        }
        assert_eq!(port_label(&port, ""), "COM6 WCH");
        if let serialport::SerialPortType::UsbPort(usb) = &mut port.port_type {
            usb.product = Some(" ".into());
        }
        assert_eq!(port_label(&port, ""), "COM6 WCH");
        assert_eq!(port.port_name, "COM6");
    }
    #[test]
    fn registry_names_and_merge() {
        assert_eq!(parameter_name("CNCA12"), Some((12, true)));
        assert_eq!(parameter_name("cncb0"), Some((0, false)));
        assert_eq!(parameter_name("COM10"), None);
        assert_eq!(device_name("\\Device\\com0com112"), Some((12, true)));
        assert_eq!(device_name("\\Device\\com0com212"), Some((12, false)));
        assert_eq!(device_name("\\Device\\com0com3"), None);
        let mut m = BTreeMap::new();
        merge(&mut m, 2, true, "COM10".into());
        merge(&mut m, 2, true, "wrong".into());
        merge(&mut m, 2, false, "CNCB2".into());
        assert_eq!(m[&2], (Some("COM10".into()), Some("CNCB2".into())));
    }
    fn installed_pairs() -> Vec<Pair> {
        vec![
            Pair {
                index: 0,
                a_name: "COM11".into(),
                b_name: "CNCB0".into(),
                b_hidden: false,
            },
            Pair {
                index: 1,
                a_name: "COM12".into(),
                b_name: "CNCB1".into(),
                b_hidden: false,
            },
        ]
    }
    #[test]
    fn actual_registry_without_b_parameter_keys() {
        let parameters = vec![
            ("CNCA1".into(), Some("COM12".into()), None),
            ("CNCA0".into(), Some("COM11".into()), None),
        ];
        let serial_comm = vec![
            ("\\Device\\com0com21".into(), "CNCB1".into()),
            ("\\Device\\com0com10".into(), "COM11".into()),
            ("\\Device\\Serial2".into(), "COM6".into()),
            ("\\Device\\com0com20".into(), "CNCB0".into()),
            ("\\Device\\com0com11".into(), "COM12".into()),
        ];
        assert_eq!(parse_pairs(&parameters, &serial_comm), installed_pairs());
        assert_eq!(parse_pairs(&parameters, &[]), installed_pairs());
        assert_eq!(parse_pairs(&[], &serial_comm), installed_pairs());
        assert!(parse_pairs(&[], &[]).is_empty());
    }
    #[test]
    fn parameter_names_win_and_default_names_are_preserved() {
        let parameters = vec![
            ("CNCA3".into(), None, None),
            ("CNCB3".into(), Some("BACKEND3".into()), None),
        ];
        let devices = vec![
            ("\\Device\\com0com13".into(), "COM99".into()),
            ("\\Device\\com0com23".into(), "CNCB3".into()),
        ];
        assert_eq!(
            parse_pairs(&parameters, &devices),
            vec![Pair {
                index: 3,
                a_name: "CNCA3".into(),
                b_name: "BACKEND3".into(),
                b_hidden: false,
            }]
        );
    }
    #[test]
    fn hidden_backends_without_serialcomm_entries() {
        for mode in [
            HiddenMode::Text("yes".into()),
            HiddenMode::Text("YES".into()),
            HiddenMode::Dword(1),
            HiddenMode::Dword(2),
            HiddenMode::Dword(0xFFFF_FFFF),
        ] {
            let parameters = vec![
                ("CNCA0".into(), Some("COM11".into()), None),
                ("CNCB0".into(), None, Some(mode)),
                ("CNCA1".into(), Some("COM12".into()), None),
                ("CNCB1".into(), None, None),
            ];
            let serial_comm = vec![
                ("\\Device\\com0com10".into(), "COM11".into()),
                ("\\Device\\com0com11".into(), "COM12".into()),
            ];
            let mut expected = installed_pairs();
            expected[0].b_hidden = true;
            assert_eq!(parse_pairs(&parameters, &serial_comm), expected);
            assert_eq!(parse_pairs(&parameters, &[]), expected);
        }
    }
    #[test]
    fn only_b_hiddenmode_yes_or_nonzero_is_hidden() {
        for mode in [
            None,
            Some(HiddenMode::Text("no".into())),
            Some(HiddenMode::Dword(0)),
            Some(HiddenMode::Text("1".into())),
        ] {
            let parameters = vec![
                (
                    "CNCA0".into(),
                    Some("COM11".into()),
                    Some(HiddenMode::Dword(1)),
                ),
                ("CNCB0".into(), None, mode),
            ];
            assert!(!parse_pairs(&parameters, &[])[0].b_hidden);
        }
    }
    #[test]
    fn setup_commands_hide_b_and_batch_deduplicates_all_targets() {
        assert_eq!(
            create_args("com14").unwrap(),
            "--silent install PortName=COM14 HiddenMode=yes"
        );
        assert!(create_args("COM14 & bad").is_err());
        assert_eq!(
            setup_commands(&Manage::HideBackends(vec![4, 0, 4])).unwrap(),
            [
                "--silent change CNCB0 HiddenMode=yes",
                "--silent change CNCB4 HiddenMode=yes"
            ]
        );
        assert!(
            setup_commands(&Manage::HideBackends(Vec::new()))
                .unwrap()
                .is_empty()
        );
        assert!(setup_commands(&Manage::Create("COM14 & bad".into())).is_err());
    }
    #[test]
    fn every_setup_action_uses_absolute_paths_cwd_and_append_log() {
        let setup = Path::new(r"C:\Program Files (x86)\com0com\setupc.exe");
        let log = Path::new(r"C:\Users\tester\AppData\Local\Temp\serial-split-setupc.log");
        for action in [
            Manage::Create("com14".into()),
            Manage::Delete(3),
            Manage::HideBackends(vec![4, 0, 4]),
        ] {
            let commands = setup_commands(&action).unwrap();
            let script = setup_script(setup, &commands, log).unwrap();
            assert!(script.starts_with(&format!(
                "cd /d \"{}\" >> \"{}\" 2>&1\r\n",
                setup.parent().unwrap().display(),
                log.display()
            )));
            assert!(!script.contains("\"setupc.exe\""));
            for command in &commands {
                assert!(script.contains(&format!(
                    "echo [%date% %time%] \"{}\" {command} >> \"{}\"\r\n",
                    setup.display(),
                    log.display()
                )));
                assert!(script.contains(&format!("\"{}\" {command} >> \"{}\" 2>&1\r\nif errorlevel 1 set \"serial_split_failed=1\"\r\n",setup.display(),log.display())));
            }
            assert_eq!(
                script.matches("if errorlevel 1 set").count(),
                commands.len()
            );
            assert!(script.ends_with("exit /b %serial_split_failed%\r\n"));
            assert!(script.contains("@if errorlevel 1 ("));
        }
        assert!(setup_script(Path::new("setupc.exe"), &[], log).is_err());
        assert!(setup_script(setup, &[], Path::new("serial-split-setupc.log")).is_err());
    }
    #[test]
    fn script_paths_escape_batch_expansion_and_launcher_supports_utf8() {
        let setup = Path::new(r"C:\工具 & 100%\com0com\setupc.exe");
        let log = Path::new(r"C:\用户\Temp\serial-split-setupc.log");
        let script =
            setup_script(setup, &setup_commands(&Manage::Delete(0)).unwrap(), log).unwrap();
        assert!(script.starts_with("cd /d \"C:\\工具 & 100%%\\com0com\""));
        assert!(script.contains("\"C:\\工具 & 100%%\\com0com\\setupc.exe\" --silent remove 0"));
        let path = Path::new(r"C:\用户\Temp\serial-split-setupc-1.cmd");
        assert_eq!(
            script_args(path),
            format!(
                "/d /s /v:off /c \"chcp 65001 >nul && \"{}\"\"",
                path.display()
            )
        );
    }
    #[test]
    fn setup_failures_expose_preserved_log_path_and_cancel_stays_clear() {
        let log = Path::new(r"C:\Temp\serial-split-setupc.log");
        let error = setup_error("setupc 返回 1".into(), log);
        assert!(error.contains("setupc 返回 1"));
        assert!(error.contains(&log.display().to_string()));
        assert_eq!(setup_error("已取消".into(), log), "已取消");
    }
    #[test]
    fn generated_script_runs_with_cmd_stub_without_elevation_or_setupc() {
        use std::os::windows::process::CommandExt;
        let cmd = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("cmd.exe");
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("脚本 & 测试-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let log = directory.join("serial-split-setupc.log");
        let script = TemporaryScript(directory.join("stub.cmd"));
        let text = setup_script(
            &cmd,
            &[
                "/d /c \"echo serial_split_first & cd & exit /b 7\"".into(),
                "/d /c \"echo serial_split_second & exit /b 0\"".into(),
            ],
            &log,
        )
        .unwrap();
        std::fs::write(&script.0, text).unwrap();
        let output = std::process::Command::new(&cmd)
            .raw_arg(script_args(&script.0))
            .current_dir(&directory)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let text =
            std::fs::read_to_string(&log).unwrap_or_else(|error| panic!("{error}; {output:?}"));
        assert!(text.lines().any(|line| line.trim() == "serial_split_first"));
        assert!(
            text.lines()
                .any(|line| line.trim() == "serial_split_second")
        );
        assert!(text.lines().any(|line| {
            line.trim()
                .eq_ignore_ascii_case(&cmd.parent().unwrap().display().to_string())
        }));
        drop(script);
        std::fs::remove_file(log).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn virtual_ports_filtered_and_test_switch_includes_only_a() {
        let info = |name: &str, port_type| serialport::SerialPortInfo {
            port_name: name.into(),
            port_type,
        };
        let available = || {
            vec![
                info("COM12", serialport::SerialPortType::Unknown),
                info("CNCB0", serialport::SerialPortType::Unknown),
                info("COM7", serialport::SerialPortType::BluetoothPort),
                info("COM6", serialport::SerialPortType::Unknown),
                info("COM11", serialport::SerialPortType::Unknown),
            ]
        };
        let normal = filter_physical_ports(available(), &installed_pairs(), false);
        assert_eq!(
            normal.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["COM6", "COM7"]
        );
        assert_eq!(normal[1].label, "COM7（蓝牙）");
        let testing = filter_physical_ports(available(), &installed_pairs(), true);
        assert_eq!(
            testing.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["COM6", "COM7", "COM11", "COM12"]
        );
        assert_eq!(testing[2].label, "COM11（虚拟）");
    }
}
