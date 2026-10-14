use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

pub fn rgba() -> Vec<u8> {
    let mut pixels = vec![0; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let at = (y * 32 + x) * 4;
            let line = (4..=15).contains(&x) && (14..=17).contains(&y)
                || (14..=17).contains(&x) && (7..=24).contains(&y)
                || (16..=27).contains(&x) && ((6..=9).contains(&y) || (22..=25).contains(&y));
            let color = if line {
                [245, 250, 255, 255]
            } else if (2..30).contains(&x) && (2..30).contains(&y) {
                [35, 105, 175, 255]
            } else {
                [0, 0, 0, 0]
            };
            pixels[at..at + 4].copy_from_slice(&color);
        }
    }
    pixels
}
pub struct Tray {
    icon: TrayIcon,
    show: MenuItem,
    exit: MenuItem,
    last_count: usize,
}
pub enum Action {
    Show,
    Exit,
}
impl Tray {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let menu = Menu::new();
        let show = MenuItem::new("显示窗口", true, None);
        let exit = MenuItem::new("退出", true, None);
        menu.append_items(&[&show, &exit])?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("serial-split：0 个分组运行中")
            .with_icon(Icon::from_rgba(rgba(), 32, 32)?)
            .build()?;
        Ok(Self {
            icon,
            show,
            exit,
            last_count: 0,
        })
    }
    pub fn poll(&self) -> Option<Action> {
        while let Ok(e) = MenuEvent::receiver().try_recv() {
            if e.id == *self.exit.id() {
                return Some(Action::Exit);
            }
            if e.id == *self.show.id() {
                return Some(Action::Show);
            }
        }
        while let Ok(e) = TrayIconEvent::receiver().try_recv() {
            if matches!(
                e,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                return Some(Action::Show);
            }
        }
        None
    }
    pub fn update(&mut self, count: usize) {
        if count != self.last_count {
            let _ = self
                .icon
                .set_tooltip(Some(format!("serial-split：{count} 个分组运行中")));
            self.last_count = count;
        }
    }
}
