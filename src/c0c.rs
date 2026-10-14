use crate::{core, winsys::wide};
use std::{collections::BTreeMap, path::PathBuf, ptr};
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
            Self::Dword(value) => *value == 1,
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
fn hide_script(indices: &[u32]) -> String {
    let mut indices = indices.to_vec();
    indices.sort_unstable();
    indices.dedup();
    let mut script = "@echo off\r\nset \"serial_split_failed=0\"\r\n".to_string();
    for index in indices {
        script.push_str(&format!("\"setupc.exe\" --silent change CNCB{index} HiddenMode=yes\r\nif errorlevel 1 set \"serial_split_failed=1\"\r\n"));
    }
    script.push_str("exit /b %serial_split_failed%\r\n");
    script
}
struct TemporaryScript(PathBuf);
impl Drop for TemporaryScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
pub fn manage(action: Manage) -> Result<(), String> {
    let setup = setup_path().ok_or("没装 com0com")?;
    let args = match action {
        Manage::Create(name) => create_args(&name)?,
        Manage::Delete(index) => format!("--silent remove {index}"),
        Manage::HideBackends(indices) => {
            if indices.is_empty() {
                return Ok(());
            }
            // ASCII script; setupc resolves in the explicitly supplied working directory.
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "serial-split-hide-{}-{stamp}.cmd",
                std::process::id()
            ));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| format!("无法创建隐藏命令：{e}"))?;
            let script = TemporaryScript(path);
            std::io::Write::write_all(&mut file, hide_script(&indices).as_bytes())
                .map_err(|e| format!("无法写入隐藏命令：{e}"))?;
            drop(file);
            let cmd = PathBuf::from(std::env::var_os("SystemRoot").ok_or("未找到 SystemRoot")?)
                .join("System32/cmd.exe");
            return elevated(
                &cmd,
                &format!("/d /s /c \"\"{}\"\"", script.0.display()),
                setup.parent().unwrap(),
            );
        }
    };
    elevated(&setup, &args, setup.parent().unwrap())
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
    fn only_b_hiddenmode_yes_or_one_is_hidden() {
        for mode in [
            None,
            Some(HiddenMode::Text("no".into())),
            Some(HiddenMode::Dword(0)),
            Some(HiddenMode::Dword(2)),
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
        let script = hide_script(&[4, 0, 4]);
        assert_eq!(script.matches("--silent change").count(), 2);
        assert!(script.contains("\"setupc.exe\" --silent change CNCB0 HiddenMode=yes\r\n"));
        assert!(script.contains("\"setupc.exe\" --silent change CNCB4 HiddenMode=yes\r\n"));
        assert_eq!(script.matches("if errorlevel 1").count(), 2);
        assert!(script.ends_with("exit /b %serial_split_failed%\r\n"));
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
