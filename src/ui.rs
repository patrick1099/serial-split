use crate::{
    bridge::{self, DisplayEvent, Runtime, State},
    c0c::{self, Pair, Port},
    core::{self, CloseAction, Group, Parity, Settings},
    recorder::Recorder,
    tray::{self, Tray},
    winsys::{self, Instance},
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::{
        atomic::Ordering,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

slint::slint! {
    import { Button, CheckBox, ComboBox, LineEdit, SpinBox, ScrollView, ListView, VerticalBox, HorizontalBox } from "std-widgets.slint";
    export struct GroupRow { id:int, name:string, port:string, status:string, dot:color }
    export struct OutputRow { name:string, label:string, checked:bool, available:bool }
    export struct PairRow { index:int, a:string, b:string, owner:string }
    export struct LogRow { text:string, ink:color }

    export component AppWindow inherits Window {
        title: "serial-split";
        preferred-width: 1120px; preferred-height: 790px;
        min-width: 920px; min-height: 680px;
        default-font-family: "Microsoft YaHei";
        background: #f5f7fa;
        in property <image> app-icon;
        icon: root.app-icon;
        in property <[GroupRow]> groups;
        in property <[OutputRow]> outputs;
        in property <[PairRow]> pairs;
        in property <[LogRow]> logs;
        in property <[string]> ports;
        in property <[string]> bauds;
        in property <[string]> parities;
        in property <[string]> close-actions;
        in property <bool> has-group;
        in property <bool> locked;
        in property <bool> stopping;
        in property <bool> desired-running;
        in property <int> selected-id;
        in-out property <string> group-name;
        in-out property <int> port-index: -1;
        in-out property <int> baud-index: 3;
        in-out property <int> bits-index: 0;
        in-out property <int> parity-index: 0;
        in-out property <int> stop-index: 0;
        in-out property <bool> recording: true;
        in property <string> state-text;
        in property <color> state-color: #64748b;
        in property <string> counters;
        in property <string> global-error;
        in property <string> detection-text;
        in-out property <bool> paused: false;
        in-out property <bool> auto-scroll: true;
        in-out property <bool> autostart: false;
        in-out property <int> close-index: 0;
        in-out property <int> retention: 30;
        in-out property <bool> close-dialog: false;
        in-out property <bool> remember-close: false;
        in-out property <bool> manage-dialog: false;
        in-out property <int> pair-index: -1;
        in-out property <string> new-port;
        in property <string> manage-message;
        in property <bool> manage-busy;
        callback select-group(int);
        callback new-group();
        callback delete-group();
        callback edit(int,string);
        callback toggle-output(string,bool);
        callback toggle-record(bool);
        callback refresh-ports();
        callback toggle-run();
        callback pause(bool);
        callback clear-log();
        callback startup(bool);
        callback close-setting(int);
        callback retention-setting(int);
        callback open-data();
        callback open-manager();
        callback manage(int);
        callback close-choice(int,bool);
        public function scroll-end() { if (root.auto-scroll && !root.paused) { log-view.content-y = min(0px, log-view.height - log-view.content-height); } }
        VerticalBox {
            padding: 14px; spacing: 10px;
            Text { text: "serial-split   串口共享与录制"; font-size: 23px; font-weight: 700; color: #16324f; }
            Text {text: root.detection-text; color: #64748b; font-size: 12px;}
            if root.global-error != "": Text { text: root.global-error; color: #c62828; wrap: word-wrap; }
            HorizontalBox {
                spacing: 14px;
                Rectangle {
                    width: 225px; background: white; border-radius: 6px;
                    VerticalBox {
                        padding: 10px;
                        Text { text: "分组"; font-size: 17px; font-weight: 600; }
                        ListView {
                            for group in root.groups: Rectangle {
                                height: 70px; background: group.id == root.selected-id ? #e7f0fb : white;
                                HorizontalBox {
                                    padding: 8px; spacing: 8px;
                                    Rectangle { width: 10px; height: 10px; border-radius: 5px; background: group.dot; }
                                    VerticalBox {
                                        spacing: 2px;
                                        Text { text: group.name; font-weight: 600; overflow: elide; }
                                        Text { text: group.port + "  " + group.status; color: #64748b; font-size: 12px; overflow: elide; }
                                    }
                                }
                                TouchArea { clicked => {root.select-group(group.id);} }
                            }
                        }
                        Button { text: "新建分组"; clicked => {root.new-group();} }
                        Button { text: "删除分组"; enabled: root.has-group && !root.locked; clicked => {root.delete-group();} }
                    }
                }
                VerticalBox {
                    spacing: 8px;
                    if !root.has-group: Text { text: "点击“新建分组”配置一个物理串口。"; color: #64748b; }
                    if root.has-group: VerticalBox {
                        spacing: 8px; vertical-stretch: 0;
                        HorizontalBox {
                            Text {text: "名称"; width: 60px;}
                            LineEdit { text <=> root.group-name; enabled: !root.locked; edited => {root.edit(0,self.text);} }
                        }
                        HorizontalBox {
                            Text {text: "物理口"; width: 60px;}
                            ComboBox { model: root.ports; current-index <=> root.port-index; enabled: !root.locked; selected(value) => {root.edit(1,value);} }
                            Button {text: "刷新"; clicked => {root.refresh-ports();} }
                        }
                        HorizontalBox {
                            Text {text: "波特率";}
                            ComboBox {model: root.bauds; current-index <=> root.baud-index; enabled: !root.locked; selected(value) => {root.edit(2,value);} }
                            Text {text: "数据位";}
                            ComboBox {model: ["8","7"]; current-index <=> root.bits-index; enabled: !root.locked; selected(value) => {root.edit(3,value);} }
                            Text {text: "校验";}
                            ComboBox {model: root.parities; current-index <=> root.parity-index; enabled: !root.locked; selected(value) => {root.edit(4,value);} }
                            Text {text: "停止位";}
                            ComboBox {model: ["1","2"]; current-index <=> root.stop-index; enabled: !root.locked; selected(value) => {root.edit(5,value);} }
                        }
                        Text {text: "转发到虚拟口（其他软件打开 A 端）"; font-weight: 600;}
                        ScrollView {
                            height: 88px;
                            VerticalBox {
                                for output in root.outputs: CheckBox {
                                    text: output.label; checked: output.checked;
                                    enabled: !root.locked && output.available;
                                    toggled => {root.toggle-output(output.name,self.checked);}
                                }
                            }
                        }
                        HorizontalBox {
                            CheckBox {text: "录制到数据库"; checked <=> root.recording; enabled: !root.locked; toggled => {root.toggle-record(self.checked);} }
                            Rectangle {horizontal-stretch: 1;}
                            Button {text: root.stopping ? "正在停止…" : root.desired-running ? "停止" : "开始"; enabled: !root.stopping; clicked => {root.toggle-run();} }
                        }
                    }
                    Text {text: root.state-text; color: root.state-color; wrap: word-wrap;}
                    Text {text: root.counters; color: #475569; font-size: 12px; wrap: word-wrap;}
                    HorizontalBox {
                        Text {text: "实时数据"; font-weight: 600;}
                        Rectangle {horizontal-stretch: 1;}
                        CheckBox {text: "暂停显示"; checked <=> root.paused; toggled => {root.pause(self.checked);} }
                        CheckBox {text: "自动滚动"; checked <=> root.auto-scroll; toggled => {root.scroll-end();} }
                        Button {text: "清屏"; clicked => {root.clear-log();} }
                    }
                    Rectangle {
                        background: white; border-width: 1px; border-color: #d6deea; vertical-stretch: 1;
                        log-view := ListView {
                            content-width: max(self.visible-width, 1650px);
                            for row in root.logs: Text {height: 23px; text: row.text; color: row.ink; font-family: "Consolas"; font-size: 12px; overflow: elide; }
                        }
                    }
                }
            }
            HorizontalBox {
                spacing: 9px;
                CheckBox {text: "开机自启"; checked <=> root.autostart; toggled => {root.startup(self.checked);} }
                Text {text: "关闭窗口";}
                ComboBox {model: root.close-actions; current-index <=> root.close-index; selected(value) => {root.close-setting(self.current-index);} }
                Text {text: "保留天数";}
                SpinBox {minimum: 1; maximum: 3650; value <=> root.retention; edited(value) => {root.retention-setting(value);} }
                Button {text: "打开数据目录"; clicked => {root.open-data();} }
                Button {text: "虚拟口管理…"; clicked => {root.open-manager();} }
            }
        }
        if root.close-dialog: Rectangle {
            background: #00000080;
            TouchArea {}
            Rectangle {
                width: 520px; height: 210px; x: (parent.width - self.width)/2; y: (parent.height - self.height)/2;
                background: white; border-radius: 8px;
                VerticalBox {
                    padding: 22px; spacing: 14px;
                    Text {text: "关闭窗口"; font-size: 22px; font-weight: 600;}
                    Text {text: "最小化后转发和录制仍继续运行。";}
                    CheckBox {text: "记住我的选择"; checked <=> root.remember-close;}
                    HorizontalBox {
                        Button {text: "最小化到托盘"; clicked => {root.close-choice(1,root.remember-close);} }
                        Button {text: "退出程序"; clicked => {root.close-choice(2,root.remember-close);} }
                        Button {text: "取消"; clicked => {root.close-choice(0,false);} }
                    }
                }
            }
        }
        if root.manage-dialog: Rectangle {
            background: #00000080;
            TouchArea {}
            Rectangle {
                width: 740px; height: 480px; x: (parent.width - self.width)/2; y: (parent.height - self.height)/2;
                background: white; border-radius: 8px;
                VerticalBox {
                    padding: 20px; spacing: 12px;
                    Text {text: "com0com 虚拟口管理"; font-size: 21px; font-weight: 600;}
                    Text {text: "A 端给其他软件使用，B 端由本程序打开。"; color: #64748b;}
                    ListView {
                        for pair[i] in root.pairs: Rectangle {
                            height: 40px; background: root.pair-index == i ? #e7f0fb : white;
                            Text {text: "#" + pair.index + "    " + pair.a + "  ⇄  " + pair.b + "    " + pair.owner; vertical-alignment: center;}
                            TouchArea {clicked => {root.pair-index=i;} }
                        }
                    }
                    Text {text: root.manage-message; color: #b45309; wrap: word-wrap;}
                    HorizontalBox {
                        LineEdit {text <=> root.new-port; enabled: !root.manage-busy;}
                        Button {text: "新建一对"; enabled: !root.manage-busy; clicked => {root.manage(0);} }
                        Button {text: "删除选中的一对"; enabled: !root.manage-busy && root.pair-index >= 0; clicked => {root.manage(1);} }
                    }
                    HorizontalBox {
                        Button {text: "重新检测"; enabled: !root.manage-busy; clicked => {root.manage(2);} }
                        Button {text: "关闭"; clicked => {root.manage-dialog=false;} }
                    }
                }
            }
        }
    }
}

type History = Rc<VecModel<LogRow>>;
enum Background {
    Detected(Vec<Pair>, Result<Vec<Port>, String>),
    Managed(Result<(), String>),
}
pub struct App {
    settings: Settings,
    settings_path: PathBuf,
    data_dir: PathBuf,
    exe: PathBuf,
    selected: Option<u32>,
    ports: Vec<Port>,
    editor_ports: Vec<Port>,
    pairs: Vec<Pair>,
    include_virtual: bool,
    displayed_group: Option<u32>,
    bridge_pairs: std::sync::Arc<std::sync::Mutex<Vec<Pair>>>,
    runtimes: HashMap<u32, Runtime>,
    stopped_stats: HashMap<u32, std::sync::Arc<bridge::Stats>>,
    histories: HashMap<u32, History>,
    last_states: HashMap<u32, String>,
    errors: HashMap<u32, String>,
    display_tx: SyncSender<DisplayEvent>,
    display_rx: Receiver<DisplayEvent>,
    background_tx: SyncSender<Background>,
    background_rx: Receiver<Background>,
    detecting: bool,
    recorder: Recorder,
    clock: Instant,
    offset: i32,
    pub instance: Instance,
    pub tray: Tray,
    global_error: String,
}

fn color(r: u8, g: u8, b: u8) -> slint::Color {
    slint::Color::from_rgb_u8(r, g, b)
}
fn strings(items: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    Rc::new(VecModel::from(
        items
            .into_iter()
            .map(SharedString::from)
            .collect::<Vec<_>>(),
    ))
    .into()
}

impl App {
    pub fn new(
        settings: Settings,
        settings_path: PathBuf,
        data_dir: PathBuf,
        exe: PathBuf,
        include_virtual: bool,
        instance: Instance,
        tray: Tray,
    ) -> Self {
        let recorder = Recorder::start(&data_dir.join("capture.sqlite3"), settings.retention_days);
        let (display_tx, display_rx) = mpsc::sync_channel(1024);
        let (background_tx, background_rx) = mpsc::sync_channel(8);
        let selected = settings.groups.first().map(|g| g.id);
        Self {
            settings,
            settings_path,
            data_dir,
            exe,
            selected,
            displayed_group: None,
            ports: Vec::new(),
            editor_ports: Vec::new(),
            pairs: Vec::new(),
            bridge_pairs: Default::default(),
            include_virtual,
            runtimes: HashMap::new(),
            stopped_stats: HashMap::new(),
            histories: HashMap::new(),
            last_states: HashMap::new(),
            errors: HashMap::new(),
            display_tx,
            display_rx,
            background_tx,
            background_rx,
            detecting: false,
            recorder,
            clock: Instant::now(),
            offset: winsys::timezone_offset(),
            instance,
            tray,
            global_error: String::new(),
        }
    }
    fn save(&mut self) {
        if let Err(e) = save_settings(&self.settings_path, &self.settings) {
            self.global_error = format!("配置保存失败：{e}");
        }
    }
    fn history(&mut self, id: u32) -> History {
        self.histories
            .entry(id)
            .or_insert_with(|| Rc::new(VecModel::default()))
            .clone()
    }
    fn push(&mut self, id: u32, line: LogRow) {
        let model = self.history(id);
        if model.row_count() >= 2000 {
            model.remove(0);
        }
        model.push(line);
    }
    fn request_detect(&mut self) {
        if self.detecting {
            return;
        }
        self.detecting = true;
        let tx = self.background_tx.clone();
        let include = self.include_virtual;
        thread::spawn(move || {
            let pairs = c0c::detect_pairs();
            let ports = c0c::physical_ports(&pairs, include);
            let _ = tx.send(Background::Detected(pairs, ports));
        });
    }
    fn outputs_model(&self) -> ModelRc<OutputRow> {
        let Some(group) = self.selected.and_then(|id| self.settings.group(id)) else {
            return ModelRc::default();
        };
        let mut names: Vec<String> = self.pairs.iter().map(|p| p.a_name.clone()).collect();
        for name in &group.outputs {
            if !names
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(name))
            {
                names.push(name.clone());
            }
        }
        Rc::new(VecModel::from(
            names
                .into_iter()
                .map(|name| {
                    let owner = self.settings.output_owner(&name, group.id);
                    let missing = !self
                        .pairs
                        .iter()
                        .any(|p| p.a_name.eq_ignore_ascii_case(&name));
                    OutputRow {
                        label: format!(
                            "{name}{}",
                            owner
                                .map(|g| format!("（被 {} 使用）", g.name))
                                .unwrap_or_else(|| if missing {
                                    "（未检测到）".into()
                                } else {
                                    String::new()
                                })
                        )
                        .into(),
                        checked: group.outputs.iter().any(|o| o.eq_ignore_ascii_case(&name)),
                        available: owner.is_none(),
                        name: name.into(),
                    }
                })
                .collect::<Vec<_>>(),
        ))
        .into()
    }
    fn pair_model(&self) -> ModelRc<PairRow> {
        Rc::new(VecModel::from(
            self.pairs
                .iter()
                .map(|p| PairRow {
                    index: p.index as i32,
                    a: p.a_name.clone().into(),
                    b: p.b_name.clone().into(),
                    owner: self
                        .settings
                        .groups
                        .iter()
                        .filter(|g| g.outputs.iter().any(|o| o.eq_ignore_ascii_case(&p.a_name)))
                        .map(|g| g.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                        .into(),
                })
                .collect::<Vec<_>>(),
        ))
        .into()
    }
    fn editor(&mut self, w: &AppWindow) {
        let group = self
            .selected
            .and_then(|id| self.settings.group(id))
            .cloned();
        w.set_has_group(group.is_some());
        w.set_selected_id(self.selected.unwrap_or(0) as i32);
        self.editor_ports = self.ports.clone();
        self.editor_ports.insert(
            0,
            Port {
                name: String::new(),
                label: "请选择物理口".into(),
            },
        );
        if let Some(g) = &group {
            if !g.port.is_empty()
                && !self
                    .editor_ports
                    .iter()
                    .any(|p| p.name.eq_ignore_ascii_case(&g.port))
            {
                self.editor_ports.push(Port {
                    name: g.port.clone(),
                    label: format!("{}（未连接）", g.port),
                });
            }
        }
        w.set_ports(strings(self.editor_ports.iter().map(|p| p.label.clone())));
        if let Some(g) = group {
            w.set_group_name(g.name.into());
            w.set_port_index(
                self.editor_ports
                    .iter()
                    .position(|p| p.name.eq_ignore_ascii_case(&g.port))
                    .map_or(-1, |i| i as i32),
            );
            let mut bauds = core::BAUDS.to_vec();
            if !bauds.contains(&g.baud) {
                bauds.push(g.baud);
            }
            w.set_bauds(strings(bauds.iter().map(ToString::to_string)));
            w.set_baud_index(bauds.iter().position(|b| *b == g.baud).unwrap() as i32);
            w.set_bits_index(if g.data_bits == 7 { 1 } else { 0 });
            w.set_parity_index(Parity::ALL.iter().position(|p| *p == g.parity).unwrap() as i32);
            w.set_stop_index(if g.stop_bits == 2 { 1 } else { 0 });
            w.set_recording(g.record);
        }
        if self.displayed_group != self.selected {
            self.bind_history(w);
        }
        w.set_outputs(self.outputs_model());
        w.set_pairs(self.pair_model());
        w.set_detection_text(if self.pairs.is_empty() {
            "没检测到 com0com 虚拟口对".into()
        } else {
            format!("检测到 {} 对虚拟口", self.pairs.len()).into()
        });
        self.status(w);
    }
    fn bind_history(&mut self, w: &AppWindow) {
        if let Some(id) = self.selected {
            if w.get_paused() {
                w.set_logs(
                    Rc::new(VecModel::from(self.history(id).iter().collect::<Vec<_>>())).into(),
                );
            } else {
                w.set_logs(self.history(id).into());
            }
        } else {
            w.set_logs(ModelRc::default());
        }
        self.displayed_group = self.selected;
    }
    fn state(&self, id: u32) -> State {
        self.runtimes
            .get(&id)
            .map(|r| r.state.lock().unwrap().clone())
            .unwrap_or_else(|| {
                self.errors
                    .get(&id)
                    .map_or(State::Stopped, |e| State::Error(e.clone()))
            })
    }
    fn state_label(state: &State) -> (String, slint::Color, &'static str) {
        match state {
            State::Running => ("运行中".into(), color(22, 163, 74), "运行"),
            State::Waiting(e) => (format!("等待重试：{e}"), color(217, 119, 6), "等待"),
            State::Stopping => ("正在停止…".into(), color(100, 116, 139), "停止中"),
            State::Stopped => ("已停止".into(), color(100, 116, 139), "停止"),
            State::Error(e) => (format!("错误：{e}"), color(198, 40, 40), "错误"),
        }
    }
    fn status(&mut self, w: &AppWindow) {
        let rows = self
            .settings
            .groups
            .iter()
            .map(|g| {
                let (_, dot, status) = Self::state_label(&self.state(g.id));
                GroupRow {
                    id: g.id as i32,
                    name: g.name.clone().into(),
                    port: g.port.clone().into(),
                    status: status.into(),
                    dot,
                }
            })
            .collect::<Vec<_>>();
        // Avoid rebuilding the left list on every idle timer tick.
        let previous = w.get_groups();
        if previous.row_count() != rows.len()
            || rows
                .iter()
                .enumerate()
                .any(|(i, row)| previous.row_data(i).as_ref() != Some(row))
        {
            w.set_groups(Rc::new(VecModel::from(rows)).into());
        }
        if let Some(id) = self.selected {
            let state = self.state(id);
            let (text, ink, _) = Self::state_label(&state);
            if self.last_states.get(&id) != Some(&text) {
                self.push(
                    id,
                    LogRow {
                        text: text.clone().into(),
                        ink: if matches!(state, State::Waiting(_) | State::Error(_)) {
                            color(198, 40, 40)
                        } else {
                            color(120, 120, 120)
                        },
                    },
                );
                self.last_states.insert(id, text.clone());
            }
            w.set_state_text(text.into());
            w.set_state_color(ink);
            let locked = self.runtimes.contains_key(&id);
            let desired = self.settings.group(id).is_some_and(|g| g.running);
            w.set_locked(locked);
            w.set_stopping(locked && !desired || matches!(state, State::Stopping));
            w.set_desired_running(desired);
            let stats = self
                .runtimes
                .get(&id)
                .map(|r| &r.stats)
                .or_else(|| self.stopped_stats.get(&id));
            w.set_counters(
                stats
                    .map(|s| {
                        let mut text = format!(
                            "物理口收 {} / 发 {}",
                            core::human_bytes(s.rx.load(Ordering::Relaxed)),
                            core::human_bytes(s.tx.load(Ordering::Relaxed))
                        );
                        for (name, c) in &s.outputs {
                            text.push_str(&format!(
                                "    {name} 转出 {} / 丢弃 {}",
                                core::human_bytes(c.sent.load(Ordering::Relaxed)),
                                core::human_bytes(c.dropped.load(Ordering::Relaxed))
                            ));
                        }
                        text.push_str(&format!(
                            "    显示丢弃 {} 条",
                            s.display_dropped.load(Ordering::Relaxed)
                        ));
                        if let Some(warning) = s.warning.lock().unwrap().as_ref() {
                            text.push_str(&format!("\n{warning}"));
                        }
                        text.into()
                    })
                    .unwrap_or_else(|| "物理口收 0 B / 发 0 B".into()),
            );
        } else {
            w.set_locked(false);
            w.set_state_text("".into());
            w.set_counters("".into());
        }
        let db_error = self
            .recorder
            .sink
            .error
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_default();
        w.set_global_error(
            [self.global_error.as_str(), db_error.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("；")
                .into(),
        );
    }
    fn start(&mut self, id: u32) {
        if self.runtimes.contains_key(&id) {
            return;
        }
        let Some(group) = self.settings.group(id).cloned() else {
            return;
        };
        let others = self
            .settings
            .groups
            .iter()
            .filter(|g| g.id != id && self.runtimes.contains_key(&g.id))
            .map(|g| g.port.clone())
            .collect::<Vec<_>>();
        let result = core::validate_start(&group, &others).and_then(|_| {
            for o in &group.outputs {
                if let Some(owner) = self.settings.output_owner(o, id) {
                    return Err(format!("{o} 被 {} 使用", owner.name));
                }
            }
            Ok(())
        });
        if let Err(e) = result {
            self.errors.insert(id, e);
            return;
        }
        self.errors.remove(&id);
        self.settings.group_mut(id).unwrap().running = true;
        self.save();
        self.runtimes.insert(
            id,
            Runtime::start(
                group,
                self.bridge_pairs.clone(),
                self.recorder.sink.clone(),
                self.display_tx.clone(),
                self.clock,
            ),
        );
    }
    pub fn initialize(&mut self, w: &AppWindow) {
        w.set_parities(strings(Parity::ALL.map(|p| p.label().to_string())));
        w.set_close_actions(strings(CloseAction::ALL.map(|c| c.label().to_string())));
        w.set_close_index(
            CloseAction::ALL
                .iter()
                .position(|c| *c == self.settings.close_action)
                .unwrap() as i32,
        );
        w.set_retention(self.settings.retention_days.min(3650) as i32);
        match winsys::refresh_autostart(&self.exe) {
            Ok(enabled) => w.set_autostart(enabled),
            Err(e) => self.global_error = format!("自启设置读取失败：{e}"),
        }
        self.editor(w);
        self.request_detect();
    }
    pub fn tick(&mut self, w: &AppWindow) {
        let mut added = false;
        while let Ok(event) = self.display_rx.try_recv() {
            self.accept_display(event);
            added = true;
        }
        while let Ok(event) = self.background_rx.try_recv() {
            match event {
                Background::Detected(pairs, ports) => {
                    self.apply_detection(pairs, ports, w);
                }
                Background::Managed(result) => {
                    w.set_manage_busy(false);
                    w.set_manage_message(
                        result
                            .err()
                            .unwrap_or_else(|| "操作完成，正在重新检测".into())
                            .into(),
                    );
                    self.request_detect();
                }
            }
        }
        let finished = self
            .runtimes
            .iter_mut()
            .filter_map(|(id, r)| r.finished().then_some(*id))
            .collect::<Vec<_>>();
        for id in finished {
            if let Some(mut r) = self.runtimes.remove(&id) {
                r.shutdown();
                self.stopped_stats.insert(id, r.stats.clone());
            }
        }
        self.status(w);
        self.tray.update(
            self.runtimes
                .values()
                .filter(|r| matches!(*r.state.lock().unwrap(), State::Running))
                .count(),
        );
        if self.instance.show_requested() {
            let _ = w.show();
            winsys::foreground();
        }
        match self.tray.poll() {
            Some(tray::Action::Show) => {
                let _ = w.show();
                winsys::foreground();
            }
            Some(tray::Action::Exit) => {
                let _ = slint::quit_event_loop();
            }
            None => {}
        }
        if added {
            w.invoke_scroll_end();
        }
    }
    pub fn accept_display(&mut self, event: DisplayEvent) {
        let error = event.notice.is_some();
        let text = event.notice.unwrap_or_else(|| {
            core::display_line(
                &core::local_hms(event.ms, self.offset),
                event.direction,
                event.source.as_deref(),
                &event.raw,
            )
        });
        self.push(
            event.group,
            LogRow {
                text: text.into(),
                ink: if error {
                    color(198, 40, 40)
                } else if event.direction == core::Direction::Rx {
                    color(25, 25, 25)
                } else {
                    color(30, 80, 190)
                },
            },
        );
    }
    pub fn apply_detection(
        &mut self,
        pairs: Vec<Pair>,
        ports: Result<Vec<Port>, String>,
        w: &AppWindow,
    ) {
        self.detecting = false;
        self.pairs = pairs;
        *self.bridge_pairs.lock().unwrap() = self.pairs.clone();
        match ports {
            Ok(p) => self.ports = p,
            Err(e) => self.global_error = format!("串口检测失败：{e}"),
        }
        self.editor(w);
        let restore = self
            .settings
            .groups
            .iter()
            .filter(|g| {
                g.running && !self.runtimes.contains_key(&g.id) && !self.errors.contains_key(&g.id)
            })
            .map(|g| g.id)
            .collect::<Vec<_>>();
        for id in restore {
            self.start(id);
        }
    }
    pub fn shutdown(&mut self) {
        for runtime in self.runtimes.values_mut() {
            runtime.shutdown();
        }
        self.recorder.shutdown();
        self.save();
    }
    pub fn request_close(&mut self, w: &AppWindow) {
        match self.settings.close_action {
            CloseAction::Ask => {
                w.set_remember_close(false);
                w.set_close_dialog(true);
            }
            CloseAction::Tray => {
                let _ = w.hide();
            }
            CloseAction::Exit => {
                let _ = slint::quit_event_loop();
            }
        }
    }
}

pub fn save_settings(path: &std::path::Path, settings: &Settings) -> std::io::Result<()> {
    use std::io::Write;
    let temp = path.with_extension("ini.tmp");
    let mut file = std::fs::File::create(&temp)?;
    file.write_all(core::format_settings(settings).as_bytes())?;
    file.sync_all()?;
    drop(file);
    winsys::replace_file(&temp, path)
}

pub fn bind(w: &AppWindow, app: Rc<RefCell<App>>) {
    let weak = w.as_weak();
    let a = app.clone();
    w.on_select_group(move |id| {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            a.selected = Some(id as u32);
            a.editor(&w);
            w.invoke_scroll_end();
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_new_group(move || {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            let id = a.settings.next_group_id();
            let name = a.settings.next_group_name();
            a.settings.groups.push(Group::new(id, name));
            a.selected = Some(id);
            a.save();
            a.editor(&w);
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_delete_group(move || {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            if let Some(id) = a.selected {
                if a.runtimes.contains_key(&id) {
                    return;
                }
                a.settings.groups.retain(|g| g.id != id);
                a.histories.remove(&id);
                a.stopped_stats.remove(&id);
                a.errors.remove(&id);
                a.last_states.remove(&id);
                a.selected = a.settings.groups.first().map(|g| g.id);
                a.save();
                a.editor(&w);
            }
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_edit(move |field, value| {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            let Some(id) = a.selected else {
                return;
            };
            if a.runtimes.contains_key(&id) {
                return;
            }
            let value = value.to_string();
            let port = a
                .editor_ports
                .iter()
                .find(|p| p.label == value)
                .map(|p| p.name.clone());
            let Some(g) = a.settings.group_mut(id) else {
                return;
            };
            match field {
                0 => g.name = value,
                1 => {
                    if let Some(p) = port {
                        g.port = p;
                    }
                }
                2 => {
                    if let Ok(b) = value.parse() {
                        g.baud = b;
                    }
                }
                3 => {
                    if let Ok(b) = value.parse() {
                        g.data_bits = b;
                    }
                }
                4 => {
                    if let Some(p) = Parity::ALL.into_iter().find(|p| p.label() == value) {
                        g.parity = p;
                    }
                }
                5 => {
                    if let Ok(b) = value.parse() {
                        g.stop_bits = b;
                    }
                }
                _ => {}
            }
            a.errors.remove(&id);
            a.save();
            w.set_outputs(a.outputs_model());
            w.set_pairs(a.pair_model());
            a.status(&w);
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_toggle_output(move |name, checked| {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            let Some(id) = a.selected else {
                return;
            };
            if a.runtimes.contains_key(&id) || a.settings.output_owner(&name, id).is_some() {
                return;
            }
            let g = a.settings.group_mut(id).unwrap();
            g.outputs.retain(|n| !n.eq_ignore_ascii_case(&name));
            if checked {
                g.outputs.push(name.to_string());
            }
            a.errors.remove(&id);
            a.save();
            w.set_outputs(a.outputs_model());
            w.set_pairs(a.pair_model());
            a.status(&w);
        }
    });
    let a = app.clone();
    w.on_toggle_record(move |record| {
        let mut a = a.borrow_mut();
        if let Some(id) = a.selected {
            if !a.runtimes.contains_key(&id) {
                a.settings.group_mut(id).unwrap().record = record;
                a.save();
            }
        }
    });
    let a = app.clone();
    w.on_refresh_ports(move || {
        a.borrow_mut().request_detect();
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_toggle_run(move || {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            if let Some(id) = a.selected {
                if a.settings.group(id).is_some_and(|g| g.running) {
                    if let Some(r) = a.runtimes.get_mut(&id) {
                        r.stop_async();
                    }
                    a.settings.group_mut(id).unwrap().running = false;
                    a.errors.remove(&id);
                    a.save();
                } else {
                    a.start(id);
                }
                a.status(&w);
            }
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_pause(move |_| {
        if let Some(w) = weak.upgrade() {
            a.borrow_mut().bind_history(&w);
            w.invoke_scroll_end();
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_clear_log(move || {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            if let Some(id) = a.selected {
                a.history(id).set_vec(Vec::new());
                if w.get_paused() {
                    w.set_logs(ModelRc::default());
                } else {
                    w.set_logs(a.history(id).into());
                }
            }
            w.invoke_scroll_end();
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_startup(move |enabled| {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            if let Err(e) = winsys::set_autostart(enabled, &a.exe) {
                a.global_error = format!("自启设置失败：{e}");
                w.set_autostart(winsys::autostart().unwrap_or(false));
            }
            a.status(&w);
        }
    });
    let a = app.clone();
    w.on_close_setting(move |index| {
        let mut a = a.borrow_mut();
        if let Some(c) = CloseAction::ALL.get(index as usize) {
            a.settings.close_action = *c;
            a.save();
        }
    });
    let a = app.clone();
    w.on_retention_setting(move |days| {
        if days > 0 {
            let mut a = a.borrow_mut();
            a.settings.retention_days = days as u32;
            a.recorder.retention.store(days as u32, Ordering::Relaxed);
            a.save();
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_open_data(move || {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            if let Err(e) = c0c::open_directory(&a.data_dir) {
                a.global_error = e;
                a.status(&w);
            }
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_open_manager(move || {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            let existing = a
                .ports
                .iter()
                .map(|p| p.name.clone())
                .chain(
                    a.pairs
                        .iter()
                        .flat_map(|p| [p.a_name.clone(), p.b_name.clone()]),
                )
                .collect::<Vec<_>>();
            w.set_new_port(core::suggest_port_name(&existing).into());
            w.set_pair_index(-1);
            w.set_manage_dialog(true);
            w.set_pairs(a.pair_model());
            w.set_manage_message(if c0c::setup_path().is_none() {
                "没装 com0com".into()
            } else {
                "新建和删除需要 Windows 管理员确认（UAC）".into()
            });
            a.request_detect();
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_manage(move |action| {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            if action == 2 {
                w.set_manage_message(if c0c::setup_path().is_none() {
                    "没装 com0com".into()
                } else {
                    "正在重新检测…".into()
                });
                a.request_detect();
                return;
            }
            if w.get_manage_busy() {
                return;
            }
            let request = if action == 0 {
                let name = match core::normalize_port_name(&w.get_new_port()) {
                    Ok(n) => n,
                    Err(e) => {
                        w.set_manage_message(e.into());
                        return;
                    }
                };
                if name != "COM#"
                    && (a.ports.iter().any(|p| p.name.eq_ignore_ascii_case(&name))
                        || a.pairs.iter().any(|p| {
                            p.a_name.eq_ignore_ascii_case(&name)
                                || p.b_name.eq_ignore_ascii_case(&name)
                        }))
                {
                    w.set_manage_message("该串口名已存在".into());
                    return;
                }
                c0c::Manage::Create(name)
            } else {
                let Some(pair) = a.pairs.get(w.get_pair_index() as usize) else {
                    return;
                };
                if let Some(owner) = a.settings.groups.iter().find(|g| {
                    g.outputs
                        .iter()
                        .any(|o| o.eq_ignore_ascii_case(&pair.a_name))
                        || g.port.eq_ignore_ascii_case(&pair.a_name)
                        || g.port.eq_ignore_ascii_case(&pair.b_name)
                }) {
                    w.set_manage_message(format!("被 {} 使用，不能删除", owner.name).into());
                    return;
                }
                c0c::Manage::Delete(pair.index)
            };
            w.set_manage_busy(true);
            w.set_manage_message("等待 Windows 确认…".into());
            let tx = a.background_tx.clone();
            thread::spawn(move || {
                let _ = tx.send(Background::Managed(c0c::manage(request)));
            });
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_close_choice(move |choice, remember| {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            w.set_close_dialog(false);
            if choice == 0 {
                return;
            }
            if remember {
                a.settings.close_action = if choice == 1 {
                    CloseAction::Tray
                } else {
                    CloseAction::Exit
                };
                a.save();
                w.set_close_index(choice);
            }
            if choice == 1 {
                let _ = w.hide();
            } else {
                let _ = slint::quit_event_loop();
            }
        }
    });
    let weak = w.as_weak();
    let a = app;
    w.window().on_close_requested(move || {
        if let Some(w) = weak.upgrade() {
            a.borrow_mut().request_close(&w);
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
}

pub fn timer(w: &AppWindow, app: Rc<RefCell<App>>) -> slint::Timer {
    let timer = slint::Timer::default();
    let weak = w.as_weak();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            if let Some(w) = weak.upgrade() {
                app.borrow_mut().tick(&w);
            }
        },
    );
    timer
}

pub fn create_window() -> Result<AppWindow, Box<dyn std::error::Error>> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;
    for font in ["C:/Windows/Fonts/msyh.ttc", "C:/Windows/Fonts/consola.ttf"] {
        let bytes = std::fs::read(font)?;
        let blob = slint::fontique_011::fontique::Blob::new(std::sync::Arc::new(bytes));
        slint::fontique_011::shared_collection().register_fonts(blob, None);
    }
    let window = AppWindow::new()?;
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(32, 32);
    pixels.make_mut_bytes().copy_from_slice(&tray::rgba());
    window.set_app_icon(slint::Image::from_rgba8(pixels));
    Ok(window)
}
