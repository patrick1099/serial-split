//! UI-only fixtures. Serial ports and setupc are never opened or invoked.
//! The running state and traffic in the snapshots are synthetic presentation data.
use serial_split::{
    bridge::DisplayEvent,
    c0c::{Pair, Port},
    core,
    tray::Tray,
    ui::{self, App, AppWindow, LogRow},
    winsys::Instance,
};
use slint::{ComponentHandle, Model, VecModel};
use std::{cell::RefCell, path::Path, rc::Rc, time::Duration};

fn snapshot(window: &slint::Window, path: &Path, expected: (u32, u32)) {
    assert_eq!(window.scale_factor(), 1.0);
    let pixels = window.take_snapshot().unwrap();
    let (width, height) = (pixels.width(), pixels.height());
    assert_eq!((width, height), expected);
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
    println!(
        "PASS snapshot {} {}x{} scale=1",
        path.display(),
        width,
        height
    );
}
fn pairs() -> Vec<Pair> {
    many_pairs(4)
}
fn many_pairs(count: u32) -> Vec<Pair> {
    (0..count)
        .map(|i| Pair {
            index: i,
            a_name: format!("COM{}", 11 + i),
            b_name: format!("CNCB{i}"),
            b_hidden: i != 3,
        })
        .collect()
}
fn ports() -> Vec<Port> {
    vec![Port {
        name: "COM6".into(),
        label: "COM6 USB-SERIAL CH340".into(),
    }]
}
fn click(w: &AppWindow, x: f32, y: f32) {
    pointer_click(w, x, y, slint::platform::PointerEventButton::Left);
}
fn pointer_click(w: &AppWindow, x: f32, y: f32, button: slint::platform::PointerEventButton) {
    let position = slint::LogicalPosition::new(x, y);
    w.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
    w.window()
        .dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button });
    w.window()
        .dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button });
}
fn key(w: &AppWindow, key: slint::platform::Key) {
    let text: slint::SharedString = key.into();
    w.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    w.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
}
fn replace_name(w: &AppWindow, text: &str) {
    w.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Control.into(),
        });
    w.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "a".into() });
    w.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "a".into() });
    w.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased {
            text: slint::platform::Key::Control.into(),
        });
    w.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.into() });
    w.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: text.into() });
    assert_eq!(w.get_rename_text(), text);
}
fn v4_smoke(w: &AppWindow, app: Rc<RefCell<App>>, dir: &Path) {
    let timer = slint::Timer::default();
    let weak = w.as_weak();
    let dir = dir.to_path_buf();
    let mut stage = 0;
    timer.start(slint::TimerMode::Repeated,Duration::from_millis(180),move || {
        let w=weak.upgrade().unwrap();app.borrow_mut().tick(&w);
        match stage {
            0=>pointer_click(&w,100.0,14.0,slint::platform::PointerEventButton::Right),
            1=> {
                assert_eq!(w.get_selected_id(),2);
                w.window().dispatch_event(slint::platform::WindowEvent::PointerMoved {position:slint::LogicalPosition::new(w.get_menu_x()+12.0,w.get_menu_y()+11.0)});
                snapshot(w.window(),&dir.join("v4-tab-menu.bmp"),(300,334));
                click(&w,w.get_menu_x()+12.0,w.get_menu_y()+11.0);
                assert!(w.get_rename_dialog());
            }
            2=> {
                snapshot(w.window(),&dir.join("v4-rename.bmp"),(300,334));
                assert!(w.get_rename_valid());
                replace_name(&w,"   ");assert!(!w.get_rename_valid());
                key(&w,slint::platform::Key::Return);assert!(w.get_rename_dialog());
                replace_name(&w,"维护口");key(&w,slint::platform::Key::Return);
                assert!(!w.get_rename_dialog());assert_eq!(w.get_groups().row_data(1).unwrap().name,"维护口");
                w.invoke_open_rename();
            }
            3=> {
                replace_name(&w,"放弃改名");key(&w,slint::platform::Key::Escape);
                assert!(!w.get_rename_dialog());assert_eq!(w.get_group_name(),"维护口");
                w.invoke_open_tab_menu(2,100.0,27.0);
                w.set_locked(true);assert!(!w.get_menu_can_delete());w.invoke_choose_tab_action(1);assert!(!w.get_delete_dialog());
                w.set_locked(false);w.set_stopping(true);assert!(!w.get_menu_can_delete());w.invoke_choose_tab_action(1);assert!(!w.get_delete_dialog());
                w.set_stopping(false);assert!(w.get_menu_can_delete());
            }
            4=> {
                click(&w,w.get_menu_x()+12.0,w.get_menu_y()+33.0);
                assert!(w.get_delete_dialog());assert_eq!(w.get_delete_id(),2);
            }
            5=> {
                snapshot(w.window(),&dir.join("v4-delete-confirm.bmp"),(300,334));
                key(&w,slint::platform::Key::Escape);assert!(!w.get_delete_dialog());
                assert_eq!(w.get_groups().row_count(),2);w.invoke_delete_group();
            }
            6=> {
                w.invoke_delete_choice(false);assert_eq!(w.get_groups().row_count(),2);
                w.invoke_delete_group();w.invoke_select_group(1);w.invoke_delete_choice(true);
                assert_eq!(w.get_groups().row_count(),1);assert_eq!(w.get_groups().row_data(0).unwrap().id,1);
                app.borrow_mut().apply_detection(many_pairs(8),Ok(ports()),&w);
                w.window().set_size(slint::PhysicalSize::new(380,480));
            }
            7=> {
                assert!((w.get_status_top()-w.get_recording_bottom()-5.0).abs()<1.0);
                snapshot(w.window(),&dir.join("v4-collapsed-large.bmp"),(380,480));
                w.invoke_expand(true);
            }
            8=>w.window().set_size(slint::PhysicalSize::new(1000,500)),
            9=> {
                assert!((w.get_status_top()-w.get_recording_bottom()-5.0).abs()<1.0);
                w.invoke_expand(false);w.invoke_delete_group();
            }
            10=> {
                assert!(w.get_delete_dialog());w.invoke_delete_choice(true);
                assert_eq!(w.get_groups().row_count(),0);assert!(!w.get_has_group());
                assert_eq!(w.get_port_index(),0);assert_eq!(w.get_group_name(),"");
                assert_eq!(w.get_logs().row_count(),0);w.window().set_size(slint::PhysicalSize::new(300,334));
            }
            11=> {
                snapshot(w.window(),&dir.join("v4-empty.bmp"),(300,334));
                click(&w,17.0,14.0);assert!(w.get_has_group());assert_eq!(w.get_groups().row_count(),1);
                assert_eq!(w.get_bauds().row_data(w.get_baud_index() as usize).unwrap(),"2400");
                w.invoke_delete_group();w.invoke_delete_choice(true);
                let stored=core::parse_settings(&std::fs::read_to_string(dir.join("v4-settings.ini")).unwrap());assert!(stored.groups.is_empty());
                println!("PASS right-click target, custom menu, blank-name guard, Enter/Esc, disabled delete, confirmation/cancel, captured deletion target, stretched layouts and empty-group recovery");
                slint::quit_event_loop().unwrap();
            }
            _=>panic!("v4 did not quit"),
        }
        stage+=1;
    });
    slint::run_event_loop_until_quit().unwrap();
    timer.stop();
}
fn wheel(w: &AppWindow, delta: f32) {
    w.window()
        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(w.get_output_x() + 20.0, w.get_output_y() + 8.0),
            delta_x: 0.0,
            delta_y: delta,
        });
}
fn fixture(w: &AppWindow) {
    let mut groups = w.get_groups().iter().collect::<Vec<_>>();
    groups[0].status = "运行".into();
    w.set_groups(Rc::new(VecModel::from(groups)).into());
    w.set_locked(true);
    w.set_desired_running(true);
    w.set_state_led(1);
    w.set_state_text("运行中".into());
    w.set_state_error(false);
    w.set_running_time("00:42:17".into());
    w.set_rx_count("18624".into());
    w.set_tx_count("3482".into());
    w.set_drop_count("0".into());
}
fn event(direction: core::Direction, source: Option<&str>, raw: &[u8], ms: i64) -> DisplayEvent {
    DisplayEvent {
        group: 1,
        ms,
        direction,
        source: source.map(str::to_string),
        raw: raw.to_vec(),
        notice: None,
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set before creating the backend or any worker so DPI is deterministic.
    unsafe {
        std::env::set_var("SLINT_SCALE_FACTOR", "1");
    }
    let dir = std::env::current_dir()?.join("target/ui-smoke");
    std::fs::create_dir_all(&dir)?;
    let v4 = std::env::args().any(|arg| arg == "--v4");
    let path = dir.join(if v4 {
        "v4-settings.ini"
    } else {
        "v2-settings.ini"
    });
    let capture_path = dir.join("capture.sqlite3");
    if std::env::args().any(|arg| arg == "--resume") {
        let restored = core::parse_settings(&std::fs::read_to_string(&path)?);
        assert_eq!(restored.theme, core::Theme::Dark);
        assert!(restored.show_data);
        assert_eq!(restored.groups.len(), 2);
        assert_eq!(restored.groups[0].name, "已改名红外口");
        assert_eq!(restored.groups[0].baud, 4800);
        assert_eq!(
            restored.size_collapsed,
            core::WindowSize {
                width: 380,
                height: 480
            }
        );
        assert_eq!(
            restored.size_expanded,
            core::WindowSize {
                width: 1000,
                height: 500
            }
        );
        let w = ui::create_window()?;
        let app = Rc::new(RefCell::new(App::new(
            restored,
            path,
            dir,
            std::env::current_exe()?,
            false,
            None::<Instance>,
            Tray::new()?,
        )));
        ui::bind(&w, app.clone())?;
        app.borrow_mut().initialize_view(&w);
        w.show()?;
        let timer = slint::Timer::default();
        let weak = w.as_weak();
        let mut stage = 0;
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(180),
            move || {
                let w = weak.upgrade().unwrap();
                match stage {
                    0 => {
                        assert!(w.get_dark());
                        assert!(w.get_show_data());
                        assert_eq!(w.window().size(), slint::PhysicalSize::new(1000, 500));
                        w.invoke_expand(false);
                    }
                    1 => {
                        assert_eq!(w.window().size(), slint::PhysicalSize::new(380, 480));
                        w.invoke_expand(true);
                    }
                    2 => {
                        assert_eq!(w.window().size(), slint::PhysicalSize::new(1000, 500));
                        slint::quit_event_loop().unwrap();
                    }
                    _ => panic!("restart did not quit"),
                }
                stage += 1;
            },
        );
        slint::run_event_loop_until_quit()?;
        timer.stop();
        app.borrow_mut().shutdown();
        println!(
            "PASS independent restart restored theme, groups, baud and both window sizes; no serial session opened"
        );
        return Ok(());
    }
    let mut settings = core::Settings::default();
    let mut g = core::Group::new(1, "红外口".into());
    g.port = "COM6".into();
    g.parity = core::Parity::Even;
    g.outputs = vec!["COM11".into(), "COM12".into()];
    let mut second = core::Group::new(2, "调试口".into());
    second.outputs = vec!["COM13".into()];
    settings.groups = vec![g, second];
    assert!(settings.groups.iter().all(|g| !g.running));
    let w = ui::create_window()?;
    let app = Rc::new(RefCell::new(App::new(
        settings,
        path.clone(),
        dir.clone(),
        std::env::current_exe()?,
        false,
        None::<Instance>,
        Tray::new()?,
    )));
    ui::bind(&w, app.clone())?;
    // Use the view initializer; production autostart registry repair is not part of a UI test.
    app.borrow_mut().initialize_view(&w);
    app.borrow_mut().apply_detection(pairs(), Ok(ports()), &w);
    w.show()?;
    if v4 {
        v4_smoke(&w, app.clone(), &dir);
        app.borrow_mut().shutdown();
        let c = rusqlite::Connection::open(capture_path)?;
        let count: i64 = c.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
        assert_eq!(count, 0);
        println!("PASS v4 shutdown; no serial session opened");
        return Ok(());
    }
    let timer = slint::Timer::default();
    let weak = w.as_weak();
    let a = app.clone();
    let mut stage = 0;
    timer.start(slint::TimerMode::Repeated,Duration::from_millis(180),move || {
        let w=weak.upgrade().unwrap();a.borrow_mut().tick(&w);
        match stage {
            0=> {
                a.borrow_mut().apply_detection(pairs(),Ok(ports()),&w);fixture(&w);
                snapshot(w.window(),&dir.join("v2-collapsed.bmp"),(300,334));w.invoke_expand(true);
            }
            1=> {
                a.borrow_mut().apply_detection(pairs(),Ok(ports()),&w);fixture(&w);
                w.get_logs().set_row_data(0,LogRow {stamp:"[13:40:58.002]".into(),text:"已打开 COM6 2400,E,8,1 会话#42".into(),kind:2});
                let ms=1_791_956_462_118;
                a.borrow_mut().accept_display(event(core::Direction::Tx,Some("COM11"),&[0xfe,0xfe,0xfe,0x68,0x20,0x26,0x09,0x07,0x09,0x15,0x19,0x68,0x01,0x0c],ms));
                a.borrow_mut().accept_display(event(core::Direction::Rx,None,&[0x68,0x20,0x26,0x09,0x07,0x09,0x15,0x19,0x68,0x81,0x31,0],ms+1200));
                a.borrow_mut().accept_display(event(core::Direction::Tx,Some("COM12"),b"AT+CSQ\r\n",ms+2400));
                a.borrow_mut().accept_display(event(core::Direction::Rx,None,b"+CSQ: 23,99\r\n",ms+2600));
                a.borrow_mut().accept_display(event(core::Direction::Rx,None,b"[DBG] freeze",ms+3000));
                for n in 0..3 {a.borrow_mut().accept_display(event(core::Direction::Rx,None,&[0x68,0x20,0x26,0x09,0x07,0x09,0x15,0x19,0x68,0x81,0x31,0],ms+4000+n*1000));}
                a.borrow_mut().tick(&w);fixture(&w);
                w.invoke_scroll_end();
                snapshot(w.window(),&dir.join("v2-expanded.bmp"),(780,334));w.invoke_open_manager();
            }
            2=> {
                a.borrow_mut().apply_detection(pairs(),Ok(ports()),&w);fixture(&w);
                let s=a.borrow().settings_window().unwrap();s.set_pair_index(1);s.invoke_select_pair(1);s.set_autostart(true);
                assert!(s.get_has_unhidden());
                snapshot(s.window(),&dir.join("v2-settings.bmp"),(300,354));
                s.invoke_manage(3);assert!(s.get_message().contains("不会调用 setupc"));
                let mut hidden=pairs();for p in &mut hidden {p.b_hidden=true;}
                a.borrow_mut().apply_detection(hidden,Ok(ports()),&w);assert!(!s.get_has_unhidden());
                a.borrow_mut().apply_detection(pairs(),Ok(ports()),&w);assert!(s.get_has_unhidden());
                s.invoke_done();w.invoke_expand(false);
            }
            3=> {fixture(&w);a.borrow_mut().request_close(&w);snapshot(w.window(),&dir.join("v2-close.bmp"),(300,334));w.invoke_close_choice(0,false);w.invoke_expand(true);w.invoke_theme_setting(1);}
            4=> {fixture(&w);snapshot(w.window(),&dir.join("v2-dark-expanded.bmp"),(780,334));}
            5=> {
                w.invoke_theme_setting(0);w.invoke_expand(false);
                a.borrow_mut().apply_detection(many_pairs(12),Ok(ports()),&w);
            }
            6=> {
                fixture(&w);assert_eq!(w.get_left_width(),290.0);
                assert_eq!(w.get_outputs().row_count(),12);assert!(w.get_output_scrollable());assert_eq!(w.get_output_page_rows(),4);
                snapshot(w.window(),&dir.join("v3-collapsed-many.bmp"),(300,334));
                wheel(&w,-40.0);assert_eq!(w.get_output_first_row(),2);
                w.set_locked(false);
                click(&w,w.get_output_x()+(w.get_output_width()-12.0)/2.0+6.0,w.get_output_y()+68.0);
                assert!(w.get_outputs().iter().find(|p|p.name=="COM22").unwrap().checked);
                w.invoke_toggle_output("COM22".into(),false);
                snapshot(w.window(),&dir.join("v3-collapsed-many-bottom.bmp"),(300,334));
                wheel(&w,40.0);assert_eq!(w.get_output_first_row(),0);
                let x=w.get_output_x()+w.get_output_width()-4.0;let y=w.get_output_y();
                let button=slint::platform::PointerEventButton::Left;
                w.window().dispatch_event(slint::platform::WindowEvent::PointerMoved {position:slint::LogicalPosition::new(x,y+3.0)});
                w.window().dispatch_event(slint::platform::WindowEvent::PointerPressed {position:slint::LogicalPosition::new(x,y+3.0),button});
                w.window().dispatch_event(slint::platform::WindowEvent::PointerMoved {position:slint::LogicalPosition::new(x,y+77.0)});
                w.window().dispatch_event(slint::platform::WindowEvent::PointerReleased {position:slint::LogicalPosition::new(x,y+77.0),button});
                assert_eq!(w.get_output_first_row(),2);
                w.window().set_size(slint::PhysicalSize::new(380,480));
            }
            7=> {
                fixture(&w);assert_eq!(w.get_left_width(),370.0);assert!(!w.get_output_scrollable());
                snapshot(w.window(),&dir.join("v3-collapsed-large.bmp"),(380,480));w.invoke_expand(true);
            }
            8=> {w.window().set_size(slint::PhysicalSize::new(1000,500));}
            9=> {
                fixture(&w);assert_eq!(w.get_left_width(),290.0);
                snapshot(w.window(),&dir.join("v3-expanded-large.bmp"),(1000,500));w.invoke_expand(false);
            }
            10=> {assert_eq!(w.window().size(),slint::PhysicalSize::new(380,480));w.invoke_expand(true);}
            11=> {
                assert_eq!(w.window().size(),slint::PhysicalSize::new(1000,500));
                let stored=core::parse_settings(&std::fs::read_to_string(&path).unwrap());
                assert_eq!(stored.size_collapsed,core::WindowSize {width:380,height:480});
                assert_eq!(stored.size_expanded,core::WindowSize {width:1000,height:500});
                w.invoke_theme_setting(1);
                println!("PASS resized layouts, complete-row wheel scrolling, last-row click, scrollbar drag and independent saved sizes");
            }
            12=> {
                w.set_locked(false);w.set_desired_running(false);
                w.invoke_toggle_run();assert!(w.get_state_reason().contains("界面预览"));w.invoke_manage(0);assert!(w.get_manage_message().contains("不会调用 setupc"));
                w.invoke_edit(2,"4800".into());w.set_rename_text("已改名红外口".into());w.invoke_rename_choice(true);
                w.set_rename_text("不保存".into());w.invoke_rename_choice(false);
                let stored=core::parse_settings(&std::fs::read_to_string(&path).unwrap());
                assert_eq!(stored.theme,core::Theme::Dark);assert!(stored.show_data);assert_eq!(stored.groups[0].name,"已改名红外口");assert_eq!(stored.groups[0].baud,4800);assert!(stored.groups.iter().all(|g|!g.running));
                w.invoke_new_group();assert_eq!(w.get_groups().row_count(),3);w.invoke_delete_group();assert!(w.get_delete_dialog());w.invoke_delete_choice(true);assert_eq!(w.get_groups().row_count(),2);w.invoke_select_group(1);
                w.set_hex(false);w.invoke_display_mode(false);let previous=w.get_logs().row_data(1).unwrap().text;
                a.borrow_mut().accept_display(event(core::Direction::Tx,Some("COM12"),"中文\r\n".as_bytes(),0));
                assert_eq!(w.get_logs().row_data(w.get_logs().row_count()-1).unwrap().text,"TX←COM12 中文..");assert_eq!(w.get_logs().row_data(1).unwrap().text,previous);
                a.borrow_mut().accept_display(event(core::Direction::Rx,None,&vec![b'A';4096],0));a.borrow_mut().tick(&w);assert!(w.get_log_width()>29000.0);
                for n in 0u32..2005 {a.borrow_mut().accept_display(event(core::Direction::Rx,None,&n.to_le_bytes(),0));}
                assert_eq!(w.get_logs().row_count(),2000);w.set_paused(true);w.invoke_pause(true);let frozen=w.get_logs().row_data(1999).unwrap().text;
                a.borrow_mut().accept_display(event(core::Direction::Rx,None,b"new",0));a.borrow_mut().apply_detection(pairs(),Ok(ports()),&w);
                assert_eq!(w.get_logs().row_data(1999).unwrap().text,frozen);
                w.invoke_clear_log();a.borrow_mut().accept_display(event(core::Direction::Rx,None,b"later",0));assert_eq!(w.get_logs().row_count(),0);w.set_paused(false);w.invoke_pause(false);assert_eq!(w.get_logs().row_count(),1);
                a.borrow_mut().apply_detection(Vec::new(),Ok(Vec::new()),&w);assert_eq!(w.get_detection_text(),"没检测到 com0com 虚拟口对");assert_eq!(w.get_ports().row_data(w.get_port_index() as usize).unwrap(),"COM6（未连接）");
                w.set_state_reason("等待重试：模拟原因".into());w.set_state_error(true);w.invoke_explain_state();assert!(w.get_logs().row_data(w.get_logs().row_count()-1).unwrap().text.contains("模拟原因"));
                println!("PASS callbacks, old port retained, text/HEX mode, 2000-line cap, pause/clear/resume and full status reason");
                a.borrow_mut().request_close(&w);w.invoke_close_choice(1,true);assert!(!w.window().is_visible());
            }
            13=> {
                assert!(!w.window().is_visible());w.show().unwrap();a.borrow_mut().request_close(&w);assert!(!w.window().is_visible());w.show().unwrap();w.invoke_close_setting(0);
                a.borrow_mut().request_close(&w);assert!(w.get_close_dialog());w.invoke_close_choice(2,true);println!("PASS close cancel/tray/exit, remembered policies and hidden event loop");
            }
            _=>panic!("event loop did not quit"),
        }
        stage+=1;
    });
    slint::run_event_loop_until_quit()?;
    timer.stop();
    app.borrow_mut().shutdown();
    let c = rusqlite::Connection::open(capture_path)?;
    let count: i64 = c.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
    assert_eq!(count, 0);
    println!("PASS shutdown; no serial session opened");
    Ok(())
}
