#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serial_split::{
    core, tray,
    ui::{self, App},
    winsys,
};
use slint::ComponentHandle;
use std::{cell::RefCell, path::PathBuf, rc::Rc};

fn start() -> Result<(), Box<dyn std::error::Error>> {
    let Some(instance) = winsys::Instance::acquire()? else {
        return Ok(());
    };
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let minimized = args.iter().any(|a| a == "--minimized");
    let include = args.iter().any(|a| a == "--include-virtual");
    let settings_dir =
        PathBuf::from(std::env::var_os("APPDATA").ok_or("APPDATA 未设置")?).join("serial-split");
    let data_dir = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA 未设置")?)
        .join("serial-split");
    std::fs::create_dir_all(&settings_dir)?;
    std::fs::create_dir_all(&data_dir)?;
    let settings_path = settings_dir.join("settings.ini");
    let settings = match std::fs::read_to_string(&settings_path) {
        Ok(s) => core::parse_settings(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => core::Settings::default(),
        Err(e) => return Err(e.into()),
    };
    let window = ui::create_window()?;
    let tray = tray::Tray::new()?;
    let app = Rc::new(RefCell::new(App::new(
        settings,
        settings_path,
        data_dir,
        std::env::current_exe()?,
        include,
        instance,
        tray,
    )));
    ui::bind(&window, app.clone())?;
    app.borrow_mut().initialize(&window);
    let timer = ui::timer(&window, app.clone());
    if !minimized {
        window.show()?;
    }
    let result = slint::run_event_loop_until_quit();
    timer.stop();
    app.borrow_mut().shutdown();
    result?;
    Ok(())
}
fn main() {
    if let Err(e) = start() {
        winsys::fatal(&e.to_string());
    }
}
