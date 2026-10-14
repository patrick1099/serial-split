//! Application callback smoke test; uses only target/ui-smoke for configuration and data.
//! No real serial ports, setupc or autostart writes are exercised.
use serial_split::{
    core,
    tray::Tray,
    ui::{self, App, AppWindow},
    winsys::Instance,
};
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, path::Path, rc::Rc, time::Duration};

fn snapshot(w: &AppWindow, path: &Path) {
    let pixels = w.window().take_snapshot().unwrap();
    let (width, height) = (pixels.width(), pixels.height());
    let mut bmp = vec![0u8; 54];
    bmp[..2].copy_from_slice(b"BM");
    bmp[2..6].copy_from_slice(&(54 + width * height * 4).to_le_bytes());
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&(width as i32).to_le_bytes());
    bmp[22..26].copy_from_slice(&(-(height as i32)).to_le_bytes());
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&32u16.to_le_bytes());
    for pixel in pixels.as_bytes().chunks_exact(4) {
        bmp.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
    }
    std::fs::write(path, bmp).unwrap();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let resume = std::env::args().any(|arg| arg == "--resume");
    let minimized = std::env::args().any(|arg| arg == "--minimized");
    let dir = std::env::current_dir()?.join("target/ui-smoke");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("settings.ini");
    let settings = if resume {
        core::parse_settings(&std::fs::read_to_string(&path)?)
    } else {
        core::Settings::default()
    };
    if resume {
        assert_eq!(settings.groups.len(), 1);
        assert_eq!(settings.groups[0].name, "UI 测试分组");
        assert_eq!(settings.groups[0].baud, 4800);
        assert_eq!(settings.groups[0].parity, core::Parity::Even);
        println!("PASS restart configuration");
    }
    let instance = Instance::acquire()?.ok_or("another serial-split instance is running")?;
    let w = ui::create_window()?;
    let app = Rc::new(RefCell::new(App::new(
        settings,
        path.clone(),
        dir.clone(),
        std::env::current_exe()?,
        false,
        instance,
        Tray::new()?,
    )));
    ui::bind(&w, app.clone());
    app.borrow_mut().initialize(&w);
    if !minimized {
        w.show()?;
    }
    let timer = slint::Timer::default();
    let weak = w.as_weak();
    let a = app.clone();
    let mut stage = 0;
    timer.start(slint::TimerMode::Repeated,Duration::from_millis(300),move || {
        let w=weak.upgrade().unwrap();a.borrow_mut().tick(&w);
        match stage {
            0=> {
                if minimized {assert!(!w.window().is_visible());println!("PASS minimized event loop with tray and no application window");w.show().unwrap();}
                if !resume {w.invoke_new_group();w.set_group_name("UI 测试分组".into());w.invoke_edit(0,"UI 测试分组".into());w.set_baud_index(4);w.invoke_edit(2,"4800".into());w.set_parity_index(1);w.invoke_edit(4,"偶".into());w.set_recording(false);w.invoke_toggle_record(false);}
                w.set_close_index(0);w.invoke_close_setting(0);
                let s=core::parse_settings(&std::fs::read_to_string(&path).unwrap());
                assert_eq!(s.groups[0].name,"UI 测试分组");assert_eq!(s.groups[0].baud,4800);assert_eq!(s.groups[0].parity,core::Parity::Even);assert!(!s.groups[0].record);
                println!("PASS group create/edit/save");
            }
            1=> {
                assert!(w.window().is_visible());assert!(!w.get_detection_text().is_empty());
                assert_eq!(w.get_port_index(),0);assert_eq!(w.get_ports().row_data(0).unwrap(),"请选择物理口");
                assert!(core::parse_settings(&std::fs::read_to_string(&path).unwrap()).groups[0].port.is_empty());
                snapshot(&w,&dir.join("main.bmp"));println!("PASS window and detection message: {}",w.get_detection_text());
                a.borrow_mut().apply_detection(Vec::new(),Ok(Vec::new()),&w);
                assert_eq!(w.get_detection_text(),"没检测到 com0com 虚拟口对");snapshot(&w,&dir.join("no-ports.bmp"));println!("PASS no com0com UI with empty detection");
                a.borrow_mut().request_close(&w);assert!(w.get_close_dialog());
                snapshot(&w,&dir.join("close.bmp"));w.invoke_close_choice(0,false);
                assert!(!w.get_close_dialog());assert!(w.window().is_visible());println!("PASS close cancel");
            }
            2=> {
                a.borrow_mut().request_close(&w);w.invoke_close_choice(1,true);assert!(!w.window().is_visible());
                assert_eq!(core::parse_settings(&std::fs::read_to_string(&path).unwrap()).close_action,core::CloseAction::Tray);println!("PASS close tray and remember");
                assert!(Instance::acquire().unwrap().is_none());a.borrow_mut().tick(&w);assert!(w.window().is_visible());println!("PASS single-instance show event");
            }
            3=> {
                a.borrow_mut().request_close(&w);assert!(!w.window().is_visible());w.show().unwrap();w.set_close_index(0);w.invoke_close_setting(0);
                w.invoke_open_manager();assert!(w.get_manage_dialog());println!("PASS manager without setupc execution: {}",w.get_manage_message());
                let event=|raw|serial_split::bridge::DisplayEvent {group:w.get_selected_id() as u32,ms:0,direction:core::Direction::Rx,source:None,raw,notice:None};
                for n in 0u32..2005 {a.borrow_mut().accept_display(event(n.to_le_bytes().to_vec()));}
                assert_eq!(w.get_logs().row_count(),2000);
                w.set_paused(true);w.invoke_pause(true);
                let frozen=w.get_logs().row_data(1999).unwrap().text;
                a.borrow_mut().accept_display(event(vec![255]));
                a.borrow_mut().apply_detection(Vec::new(),Ok(Vec::new()),&w);
                assert_eq!(w.get_logs().row_data(1999).unwrap().text,frozen);
                w.invoke_clear_log();a.borrow_mut().accept_display(event(vec![1,2,3]));assert_eq!(w.get_logs().row_count(),0);
                w.set_paused(false);w.invoke_pause(false);assert_eq!(w.get_logs().row_count(),1);
                println!("PASS 2000-line cap and display pause/clear/resume through detection refresh");
            }
            4=> {
                snapshot(&w,&dir.join("manager.bmp"));w.set_manage_dialog(false);
                a.borrow_mut().request_close(&w);assert!(w.get_close_dialog());w.invoke_close_choice(2,true);println!("PASS close exit and remember");
            }
            _=>panic!("event loop did not exit"),
        }
        stage+=1;
    });
    slint::run_event_loop_until_quit()?;
    timer.stop();
    app.borrow_mut().shutdown();
    println!("PASS shutdown and database flush");
    Ok(())
}
