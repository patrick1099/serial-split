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
    import { ScrollView, ListView } from "std-widgets.slint";
    export global Theme {
        in-out property <bool> dark: false;
        out property <color> face: dark ? #2b2b2b : #f0f0f0;
        out property <color> field: dark ? #1e1e1e : #ffffff;
        out property <color> text: dark ? #dcdcdc : #000000;
        out property <color> dim: dark ? #a0a0a0 : #6d6d6d;
        out property <color> disabled: dark ? #747474 : #9a9a9a;
        out property <color> edge: dark ? #555555 : #a0a0a0;
        out property <color> control-edge: dark ? #555555 : #7a7a7a;
        out property <color> group-edge: dark ? #555555 : #c8c8c8;
        out property <color> highlight: dark ? #383838 : #ffffff;
        out property <color> title: dark ? #6cb6ff : #003399;
        out property <color> selection: #0078d7;
        out property <color> selected-text: #ffffff;
        out property <color> button-top: dark ? #414141 : #f6f6f6;
        out property <color> button-bottom: dark ? #303030 : #dddddd;
        out property <color> tab: dark ? #353535 : #e6e6e6;
        out property <color> default-edge: dark ? #6cb6ff : #0078d7;
        out property <color> default-inner: dark ? #4778a0 : #9ccaf0;
        out property <color> tx: dark ? #6cb6ff : #0000c8;
        out property <color> rx: text;
        out property <color> notice: dark ? #80cf80 : #008000;
        out property <color> error: dark ? #ff8585 : #c80000;
        out property <color> timestamp: dark ? #9a9a9a : #808080;
        out property <color> led-green: #1bb31b;
        out property <color> led-green-hi: #9dff9d;
        out property <color> led-red: #d11b1b;
        out property <color> led-red-hi: #ff9d9d;
        out property <color> led-grey: #8c8c8c;
        out property <color> led-grey-hi: #e0e0e0;
        out property <color> led-edge: #696969;
        out property <color> overlay: #00000050;
    }
    component GroupBox inherits Rectangle {
        in property <string> title;
        Rectangle { y: 7px; height: parent.height - 7px; border-width: 1px; border-color: Theme.group-edge;
            Rectangle { x: 1px; y: 1px; width: parent.width - 2px; height: parent.height - 2px; border-width: 1px; border-color: Theme.highlight; }
        }
        Rectangle {x: 6px; y: 0px; width: legend.preferred-width + 6px; height: 16px; background: Theme.face;
            legend := Text {x: 3px; text: root.title; color: Theme.title; font-size: 12px;}
        }
        Rectangle { x: 7px; y: 18px; width: parent.width - 14px; height: parent.height - 24px; @children }
    }
    component Led inherits Rectangle {
        in property <int> kind: 0;
        width: 8px; height: 8px; border-radius: 4px; border-width: 1px; border-color: Theme.led-edge;
        background: root.kind == 1 ? Theme.led-green : root.kind == 2 ? Theme.led-red : Theme.led-grey;
        Rectangle { x: 2px; y: 1px; width: 3px; height: 3px; border-radius: 2px; background: root.kind == 1 ? Theme.led-green-hi : root.kind == 2 ? Theme.led-red-hi : Theme.led-grey-hi; }
    }
    component ClassicButton inherits Rectangle {
        in property <string> text;
        in property <bool> enabled: true;
        in property <bool> primary: false;
        in property <bool> with-led: false;
        in property <int> led: 0;
        callback clicked;
        height: 24px;
        border-width: 1px; border-color: root.primary ? Theme.default-edge : Theme.edge;
        background: touch.pressed ? Theme.button-bottom : @linear-gradient(180deg, Theme.button-top, Theme.button-bottom);
        if root.primary: Rectangle {x: 1px; y: 1px; width: parent.width - 2px; height: parent.height - 2px; border-width: 1px; border-color: Theme.default-inner;}
        accessible-role: button; accessible-label: root.text; accessible-enabled: root.enabled;
        HorizontalLayout { padding: 4px; spacing: 6px; alignment: center;
            if root.with-led: Led {kind: root.led; y: (parent.height - self.height)/2;}
            Text {text: root.text; color: root.enabled ? Theme.text : Theme.disabled; font-size: 12px; vertical-alignment: center;}
        }
        focus := FocusScope { enabled: root.enabled; key-pressed(e) => { if (e.text == Key.Return || e.text == " ") {root.clicked(); return accept;} return reject; } }
        touch := TouchArea { enabled: root.enabled; clicked => {focus.focus();root.clicked();} }
    }
    component ClassicCheck inherits Rectangle {
        in property <string> text;
        in property <string> suffix;
        in property <bool> enabled: true;
        in-out property <bool> checked: false;
        callback toggled(bool);
        height: 18px;
        accessible-role: checkbox; accessible-label: root.text; accessible-checked: root.checked; accessible-enabled: root.enabled;
        Rectangle { x: 0px; y: 3px; width: 12px; height: 12px; background: Theme.field; border-width: 1px; border-color: root.enabled ? Theme.control-edge : Theme.edge;
            if root.checked: Path {x: 2px; y: 3px; width: 8px; height: 6px; stroke: root.enabled ? Theme.text : Theme.disabled; stroke-width: 1.5px; commands: "M 0 3 L 3 6 L 8 0";}
        }
        HorizontalLayout { x: 16px; spacing: 3px; alignment: start;
            Text {text: root.text; color: root.enabled ? Theme.text : Theme.disabled; font-size: 12px; vertical-alignment: center;}
            Text {text: root.suffix; color: Theme.dim; font-size: 10px; vertical-alignment: center; overflow: elide;}
        }
        focus := FocusScope { enabled: root.enabled; key-pressed(e) => { if (e.text == " ") {root.checked = !root.checked;root.toggled(root.checked);return accept;} return reject; } }
        TouchArea { enabled: root.enabled; clicked => {focus.focus();root.checked = !root.checked;root.toggled(root.checked);} }
    }
    component ClassicInput inherits Rectangle {
        in-out property <string> text;
        in property <bool> enabled: true;
        callback edited(string);
        height: 20px; border-width: 1px; border-color: Theme.control-edge; background: Theme.field;
        input := TextInput {x: 4px; width: parent.width - 8px; text <=> root.text; enabled: root.enabled; single-line: true; font-size: 12px; color: root.enabled ? Theme.text : Theme.disabled; selection-background-color: Theme.selection; selection-foreground-color: Theme.selected-text; vertical-alignment: center; edited => {root.edited(self.text);} }
        forward-focus: input;
    }
    component ClassicCombo inherits Rectangle {
        in property <[string]> model;
        in-out property <int> current-index: 0;
        in property <bool> enabled: true;
        callback selected(string);
        callback opening;
        height: 20px; background: Theme.field; border-width: 1px; border-color: Theme.control-edge;
        accessible-role: combobox; accessible-enabled: root.enabled; accessible-value: root.current-index >= 0 ? root.model[root.current-index] : "";
        Text {x: 4px; width: parent.width - 22px; text: root.current-index >= 0 ? root.model[root.current-index] : ""; color: root.enabled ? Theme.text : Theme.disabled; font-size: 12px; vertical-alignment: center; overflow: elide;}
        Rectangle {width: 17px; x: parent.width - self.width - 1px; y: 1px; height: parent.height - 2px; background: Theme.face; border-width: 1px; border-color: Theme.group-edge;
            Text {text: "▼"; font-size: 8px; color: root.enabled ? Theme.text : Theme.disabled; horizontal-alignment: center; vertical-alignment: center;}
        }
        focus := FocusScope {enabled: root.enabled;
            key-pressed(e) => {
                if (e.text == Key.DownArrow || e.text == Key.UpArrow) {root.current-index = clamp(root.current-index + (e.text == Key.DownArrow ? 1 : -1), 0, root.model.length - 1);root.selected(root.model[root.current-index]);return accept;}
                if (e.text == Key.Return || e.text == " ") {root.opening(); popup.show();return accept;} return reject;
            }
        }
        TouchArea {enabled: root.enabled; clicked => {focus.focus();root.opening();popup.show();} }
        popup := PopupWindow {
            x: 0px; y: root.height; width: root.width; height: min(root.model.length, 8) * 20px + 2px; close-policy: close-on-click-outside;
            Rectangle {background: Theme.field; border-width: 1px; border-color: Theme.control-edge;
                ListView {x: 1px; y: 1px; width: parent.width - 2px; height: parent.height - 2px;
                    for value[i] in root.model: Rectangle {height: 20px; background: i == root.current-index ? Theme.selection : Theme.field;
                        Text {x: 4px; text: value; color: i == root.current-index ? Theme.selected-text : Theme.text; font-size: 12px; vertical-alignment: center; overflow: elide;}
                        TouchArea {clicked => {root.current-index = i;root.selected(value);popup.close();} }
                    }
                }
            }
        }
    }
    component TabItem inherits Rectangle {
        in property <string> text;
        in property <bool> selected;
        in property <int> led: -1;
        callback clicked;
        callback double-clicked;
        width: label.preferred-width + (root.led >= 0 ? 32px : 18px); height: 23px;
        background: root.selected ? Theme.face : Theme.tab; border-width: 1px; border-color: Theme.edge;
        if root.led >= 0: Led {x: 8px; y: 8px; kind: root.led;}
        label := Text {x: root.led >= 0 ? 21px : 8px; text: root.text; color: Theme.text; font-size: 12px; vertical-alignment: center;}
        if root.selected: Rectangle {x: 1px; y: parent.height - 1px; width: parent.width - 2px; height: 1px; background: Theme.face;}
        TouchArea {clicked => {root.clicked();} double-clicked => {root.double-clicked();} }
    }
    component StatusCell inherits Rectangle {
        in property <string> text;
        in property <bool> error;
        in property <bool> grow: false;
        callback clicked;
        min-width: label.preferred-width + 10px;
        max-width: root.grow ? 20000px : label.preferred-width + 10px;
        horizontal-stretch: root.grow ? 1 : 0;
        border-width: 1px; border-color: Theme.edge;
        Rectangle {x: 1px; y: 1px; width: parent.width - 1px; height: parent.height - 1px; border-width: 1px; border-color: Theme.highlight;}
        label := Text {x: 5px; text: root.text; color: root.error ? Theme.error : Theme.text; font-size: 12px; vertical-alignment: center;}
        TouchArea {clicked => {root.clicked();} }
    }
    export struct GroupRow { id:int, name:string, port:string, status:string }
    export struct OutputRow { name:string, label:string, suffix:string, checked:bool, available:bool }
    export struct PairRow { index:int, a:string, b:string, owner:string }
    export struct LogRow { stamp:string, text:string, kind:int }
    export component AppWindow inherits Window {
        title: "串口分路器"; default-font-family: "Microsoft YaHei"; default-font-size: 12px; background: Theme.face;
        preferred-width: root.show-data ? 780px : 300px; preferred-height: 334px;
        min-width: root.show-data ? 780px : 300px; max-width: root.show-data ? 20000px : 300px;
        min-height: 334px; max-height: root.show-data ? 20000px : 334px;
        in property <image> app-icon; icon: root.app-icon;
        in property <[GroupRow]> groups; in property <[OutputRow]> outputs; in property <[PairRow]> pairs;
        in property <[LogRow]> logs; in property <[string]> ports; in property <[string]> bauds; in property <[string]> parities; in property <[string]> close-actions;
        in property <length> log-width: 1650px;
        in property <bool> has-group; in property <bool> locked; in property <bool> stopping; in property <bool> desired-running;
        in property <int> selected-id; in-out property <string> group-name;
        in-out property <int> port-index: 0; in-out property <int> baud-index: 3; in-out property <int> bits-index: 0; in-out property <int> parity-index: 0; in-out property <int> stop-index: 0;
        in-out property <bool> recording: true; in property <string> state-text; in property <string> state-reason;
        in property <int> state-led; in property <bool> state-error; in property <string> running-time;
        in property <string> rx-count: "0"; in property <string> tx-count: "0"; in property <string> drop-count: "0";
        in property <string> global-error; in property <string> detection-text;
        in-out property <bool> paused: false; in-out property <bool> auto-scroll: true; in-out property <bool> hex: true;
        in-out property <bool> show-data: false; in-out property <bool> dark: false;
        in-out property <bool> autostart: false; in-out property <int> close-index: 0; in-out property <int> retention: 30;
        in-out property <bool> close-dialog: false; in-out property <bool> remember-close: false;
        in-out property <bool> rename-dialog: false; in-out property <string> rename-text;
        in-out property <bool> manage-dialog: false; in-out property <int> pair-index: -1; in-out property <string> new-port;
        in property <string> manage-message; in property <bool> manage-busy;
        callback select-group(int); callback new-group(); callback delete-group(); callback rename-choice(bool);
        callback edit(int,string); callback toggle-output(string,bool); callback toggle-record(bool); callback refresh-ports(); callback toggle-run();
        callback pause(bool); callback clear-log(); callback display-mode(bool); callback expand(bool); callback theme-setting(int);
        callback startup(bool); callback close-setting(int); callback retention-setting(int); callback open-data(); callback open-manager(); callback manage(int); callback close-choice(int,bool); callback explain-state();
        public function scroll-end() { if (root.auto-scroll && !root.paused) {log-view.content-y = min(0px, log-view.height - log-view.content-height);} }
        VerticalLayout {
            spacing: 0px;
            Rectangle {height: 27px;
                Rectangle {y: parent.height - 1px; height: 1px; background: Theme.edge;}
                ScrollView {x: 4px; y: 4px; width: parent.width - 8px; height: 23px;
                    HorizontalLayout {spacing: 0px; alignment: start;
                        for group in root.groups: TabItem {text: group.name; selected: group.id == root.selected-id; led: group.status == "运行" ? 1 : group.status == "等待" || group.status == "错误" ? 2 : 0;
                            clicked => {root.select-group(group.id);} double-clicked => {root.select-group(group.id);root.rename-text = root.group-name;root.rename-dialog = true;}
                        }
                        TabItem {text: "＋"; clicked => {root.new-group();} }
                    }
                }
            }
            HorizontalLayout {
                padding: 5px; spacing: root.show-data ? 5px : 0px;
                VerticalLayout {width: 290px; spacing: 5px; alignment: start;
                    GroupBox {title: "物理口"; height: 98px;
                        VerticalLayout {spacing: 4px;
                            HorizontalLayout {spacing: 6px;
                                Text {text: "端口号"; width: 52px; color: Theme.text; vertical-alignment: center;}
                                ClassicCombo {model: root.ports; current-index <=> root.port-index; enabled: root.has-group && !root.locked; opening => {root.refresh-ports();} selected(value) => {root.edit(1,value);} }
                            }
                            HorizontalLayout {spacing: 6px;
                                Text {text: "波特率"; width: 52px; color: Theme.text; vertical-alignment: center;}
                                ClassicCombo {width: 74px; model: root.bauds; current-index <=> root.baud-index; enabled: root.has-group && !root.locked; selected(value) => {root.edit(2,value);} }
                                Text {text: "校验位"; width: 44px; color: Theme.text; vertical-alignment: center;}
                                ClassicCombo {model: root.parities; current-index <=> root.parity-index; enabled: root.has-group && !root.locked; selected(value) => {root.edit(4,value);} }
                            }
                            HorizontalLayout {spacing: 6px;
                                Text {text: "数据位"; width: 52px; color: Theme.text; vertical-alignment: center;}
                                ClassicCombo {width: 74px; model: ["8","7"]; current-index <=> root.bits-index; enabled: root.has-group && !root.locked; selected(value) => {root.edit(3,value);} }
                                Text {text: "停止位"; width: 44px; color: Theme.text; vertical-alignment: center;}
                                ClassicCombo {model: ["1","2"]; current-index <=> root.stop-index; enabled: root.has-group && !root.locked; selected(value) => {root.edit(5,value);} }
                            }
                        }
                    }
                    GroupBox {title: "转发到虚拟口"; height: 72px;
                        if root.detection-text == "没检测到 com0com 虚拟口对": Text {text: root.detection-text; color: Theme.error; font-size: 12px; vertical-alignment: center;}
                        if root.detection-text != "没检测到 com0com 虚拟口对": ScrollView {
                            Rectangle {height: ceil(root.outputs.length / 2) * 20px;
                                for output[i] in root.outputs: ClassicCheck {x: mod(i, 2) * (parent.width / 2); y: floor(i / 2) * 20px; width: parent.width / 2 - 4px; text: output.name; suffix: output.suffix; checked: output.checked; enabled: root.has-group && !root.locked && output.available; toggled(value) => {root.toggle-output(output.name,value);} }
                            }
                        }
                    }
                    HorizontalLayout {spacing: 5px;
                        ClassicButton {width: 142.5px; text: root.stopping ? "停止中…" : root.desired-running ? "关闭串口" : "打开串口"; with-led: true; led: root.state-led; primary: true; enabled: root.has-group && !root.stopping; clicked => {root.toggle-run();} }
                        ClassicButton {width: 142.5px; text: root.show-data ? "<< 收起" : "显示数据 >>"; clicked => {root.expand(!root.show-data);} }
                    }
                    ClassicCheck {text: "录制到数据库"; checked <=> root.recording; enabled: root.has-group && !root.locked; toggled(value) => {root.toggle-record(value);} }
                    if !root.has-group: Text {text: "点击 ＋ 新建分组"; color: Theme.dim;}
                    if root.global-error != "": Text {text: root.global-error; color: Theme.error; wrap: word-wrap;}
                }
                GroupBox {title: "数据"; visible: root.show-data; min-width: root.show-data ? 475px : 0px; max-width: root.show-data ? 20000px : 0px; horizontal-stretch: 1;
                    VerticalLayout {spacing: 4px;
                        Rectangle {background: Theme.field; border-width: 1px; border-color: Theme.control-edge; vertical-stretch: 1;
                            log-view := ListView {x: 4px; y: 2px; width: parent.width - 8px; height: parent.height - 4px; content-width: max(self.visible-width, root.log-width);
                                for row in root.logs: HorizontalLayout {height: 17px; spacing: 7px; alignment: start;
                                    Text {text: row.stamp; font-family: "Consolas"; font-size: 12px; color: Theme.timestamp;}
                                    Text {text: row.text; font-family: "Consolas"; font-size: 12px; color: row.kind == 1 ? Theme.tx : row.kind == 2 ? Theme.notice : row.kind == 3 ? Theme.error : Theme.rx;}
                                }
                            }
                        }
                        HorizontalLayout {spacing: 10px; height: 20px;
                            ClassicCheck {width: 45px; text: "暂停"; checked <=> root.paused; toggled(value) => {root.pause(value);} }
                            ClassicCheck {width: 70px; text: "自动滚动"; checked <=> root.auto-scroll; toggled => {root.scroll-end();} }
                            ClassicCheck {width: 44px; text: "HEX"; checked <=> root.hex; toggled(value) => {root.display-mode(value);} }
                            Rectangle {horizontal-stretch: 1;}
                            ClassicButton {width: 50px; text: "清空"; height: 20px; clicked => {root.clear-log();} }
                        }
                    }
                }
            }
            Rectangle {height: 22px;
                HorizontalLayout {padding: 2px; spacing: 2px;
                    StatusCell {text: root.show-data && root.running-time != "" ? root.state-text + " " + root.running-time : root.state-text; error: root.state-error; clicked => {root.explain-state();} }
                    StatusCell {text: "RX:" + root.rx-count;}
                    StatusCell {text: "TX:" + root.tx-count;}
                    StatusCell {text: "丢:" + root.drop-count; grow: true;}
                    StatusCell {text: "设置"; clicked => {root.open-manager();} }
                }
            }
        }
        if root.rename-dialog: Rectangle {background: Theme.overlay; TouchArea {}
            Rectangle {width: 282px; height: 124px; x: (parent.width - self.width)/2; y: (parent.height - self.height)/2; background: Theme.face; border-width: 1px; border-color: Theme.control-edge;
                VerticalLayout {padding: 10px; spacing: 8px;
                    Text {text: "分组名称"; color: Theme.text;}
                    ClassicInput {text <=> root.rename-text; enabled: !root.locked;}
                    HorizontalLayout {spacing: 5px;
                        ClassicButton {text: "删除分组"; enabled: !root.locked; clicked => {root.rename-dialog = false;root.delete-group();} }
                        ClassicButton {text: "确定"; primary: true; clicked => {root.rename-choice(true);} }
                        ClassicButton {text: "取消"; clicked => {root.rename-choice(false);} }
                    }
                }
            }
        }
        if root.close-dialog: Rectangle {background: Theme.overlay; TouchArea {}
            Rectangle {width: 284px; height: 114px; x: (parent.width - self.width)/2; y: (parent.height - self.height)/2; background: Theme.face; border-width: 1px; border-color: Theme.control-edge;
                VerticalLayout {padding: 9px; spacing: 9px;
                    Text {text: "关闭窗口时："; color: Theme.text;}
                    HorizontalLayout {spacing: 4px;
                        ClassicButton {width: 110px; text: "最小化到托盘"; clicked => {root.close-choice(1,root.remember-close);} }
                        ClassicButton {text: "退出程序"; clicked => {root.close-choice(2,root.remember-close);} }
                        ClassicButton {width: 50px; text: "取消"; clicked => {root.close-choice(0,false);} }
                    }
                    ClassicCheck {text: "记住我的选择"; checked <=> root.remember-close;}
                }
            }
        }
    }
    export component SettingsWindow inherits Window {
        title: "设置"; default-font-family: "Microsoft YaHei"; default-font-size: 12px; background: Theme.face;
        preferred-width: 300px; preferred-height: 354px; min-width: 300px; max-width: 300px; min-height: 354px; max-height: 354px;
        in property <image> app-icon; icon: root.app-icon;
        in property <[PairRow]> pairs; in-out property <int> pair-index: -1;
        in-out property <int> theme-index: 0; in-out property <int> close-index: 0; in property <[string]> close-actions;
        in-out property <string> retention-text: "30"; in-out property <bool> autostart;
        in property <string> message; in property <bool> busy; in property <bool> has-unhidden;
        in-out property <bool> new-dialog: false; in-out property <string> new-port;
        callback theme-setting(int); callback close-setting(int); callback retention-input(string); callback startup(bool); callback select-pair(int);
        callback create-pair(); callback manage(int); callback open-data(); callback done();
        VerticalLayout {padding: 6px; spacing: 6px;
            GroupBox {title: "常规"; height: 120px;
                VerticalLayout {spacing: 4px;
                    HorizontalLayout {spacing: 6px; Text {text: "外观"; width: 74px; color: Theme.text; vertical-alignment: center;} ClassicCombo {model: ["浅色","深色"]; current-index <=> root.theme-index; selected => {root.theme-setting(self.current-index);} } }
                    HorizontalLayout {spacing: 6px; Text {text: "关闭窗口时"; width: 74px; color: Theme.text; vertical-alignment: center;} ClassicCombo {model: root.close-actions; current-index <=> root.close-index; selected => {root.close-setting(self.current-index);} } }
                    HorizontalLayout {spacing: 6px; Text {text: "数据保留"; width: 74px; color: Theme.text; vertical-alignment: center;} ClassicInput {text <=> root.retention-text; edited(value) => {root.retention-input(value);} } Text {text: "天"; width: 14px; color: Theme.text; vertical-alignment: center;} }
                    ClassicCheck {text: "开机自启"; checked <=> root.autostart; toggled(value) => {root.startup(value);} }
                }
            }
            GroupBox {title: "虚拟口对（com0com）"; vertical-stretch: 1;
                VerticalLayout {spacing: 5px;
                    Rectangle {background: Theme.field; border-width: 1px; border-color: Theme.control-edge; vertical-stretch: 1;
                        VerticalLayout {padding: 1px; spacing: 0px;
                            Rectangle {height: 19px; background: Theme.face;
                                HorizontalLayout {padding-left: 4px; spacing: 0px; Text {width: 84px; text: "软件用"; color: Theme.text;} Text {width: 84px; text: "本程序用"; color: Theme.text;} Text {horizontal-stretch: 1; text: "分组"; color: Theme.text;} }
                            }
                            ListView {for pair[i] in root.pairs: Rectangle {height: 19px; background: root.pair-index == i ? Theme.selection : Theme.field;
                                HorizontalLayout {padding-left: 4px; spacing: 0px; Text {width: 84px; text: pair.a; overflow: elide; color: root.pair-index == i ? Theme.selected-text : Theme.text; font-family: "Consolas";} Text {width: 84px; text: pair.b; overflow: elide; color: root.pair-index == i ? Theme.selected-text : Theme.text; font-family: "Consolas";} Text {horizontal-stretch: 1; text: pair.owner == "" ? "—" : pair.owner; color: root.pair-index == i ? Theme.selected-text : Theme.text; overflow: elide;} }
                                TouchArea {clicked => {root.pair-index = i;root.select-pair(i);} }
                            } }
                        }
                    }
                    if root.has-unhidden: HorizontalLayout {alignment: end;
                        ClassicButton {width: 112px; text: "隐藏本程序端"; enabled: !root.busy; clicked => {root.manage(3);} }
                    }
                    HorizontalLayout {spacing: 5px; alignment: end;
                        ClassicButton {width: 48px; text: "新建"; enabled: !root.busy; clicked => {root.create-pair();} }
                        ClassicButton {width: 48px; text: "删除"; enabled: !root.busy && root.pair-index >= 0; clicked => {root.manage(1);} }
                        ClassicButton {width: 70px; text: "数据目录"; clicked => {root.open-data();} }
                    }
                }
            }
            if root.message != "": Text {text: root.message; color: Theme.error; wrap: word-wrap; font-size: 11px;}
            HorizontalLayout {alignment: end; ClassicButton {width: 70px; text: "确定"; primary: true; clicked => {root.done();} } }
        }
        if root.new-dialog: Rectangle {background: Theme.overlay; TouchArea {}
            Rectangle {x: 9px; y: (parent.height - self.height)/2; width: 282px; height: 115px; background: Theme.face; border-width: 1px; border-color: Theme.control-edge;
                VerticalLayout {padding: 10px; spacing: 8px;
                    Text {text: "新建虚拟口（软件使用的 A 端名称）"; color: Theme.text;}
                    ClassicInput {text <=> root.new-port; enabled: !root.busy;}
                    HorizontalLayout {alignment: end; spacing: 5px; ClassicButton {width: 60px; text: "确定"; primary: true; enabled: !root.busy; clicked => {root.manage(0);} } ClassicButton {width: 60px; text: "取消"; clicked => {root.new-dialog = false;} } }
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
    history_widths: HashMap<u32, f32>,
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
    pub instance: Option<Instance>,
    pub tray: Tray,
    global_error: String,
    settings_window: Option<SettingsWindow>,
    hex_mode: bool,
    running_since: HashMap<u32, Instant>,
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
        instance: impl Into<Option<Instance>>,
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
            history_widths: HashMap::new(),
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
            instance: instance.into(),
            tray,
            global_error: String::new(),
            settings_window: None,
            hex_mode: true,
            running_since: HashMap::new(),
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
        let width = 120.0
            + line
                .text
                .chars()
                .map(|c| if c.is_ascii() { 7.2 } else { 14.0 })
                .sum::<f32>();
        let widest = self.history_widths.entry(id).or_default();
        *widest = widest.max(width);
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
                        label: name.clone().into(),
                        suffix: owner
                            .map(|g| g.name.clone())
                            .unwrap_or_else(|| {
                                if missing {
                                    "（未检测到）".into()
                                } else {
                                    String::new()
                                }
                            })
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
            w.set_parity_index(
                [Parity::None, Parity::Odd, Parity::Even]
                    .iter()
                    .position(|p| *p == g.parity)
                    .unwrap() as i32,
            );
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
        w.set_log_width(
            self.selected
                .and_then(|id| self.history_widths.get(&id))
                .copied()
                .unwrap_or(0.0),
        );
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
    fn state_label(state: &State) -> (String, &'static str) {
        match state {
            State::Running => ("运行中".into(), "运行"),
            State::Waiting(e) => (format!("等待重试：{e}"), "等待"),
            State::Stopping => ("停止中".into(), "停止中"),
            State::Stopped => ("已停止".into(), "停止"),
            State::Error(e) => (format!("错误：{e}"), "错误"),
        }
    }
    fn status(&mut self, w: &AppWindow) {
        if !w.get_paused() {
            w.set_log_width(
                self.selected
                    .and_then(|id| self.history_widths.get(&id))
                    .copied()
                    .unwrap_or(0.0),
            );
        }
        for group in &self.settings.groups {
            if matches!(self.state(group.id), State::Running) {
                self.running_since
                    .entry(group.id)
                    .or_insert_with(Instant::now);
            } else {
                self.running_since.remove(&group.id);
            }
        }
        let rows = self
            .settings
            .groups
            .iter()
            .map(|g| {
                let (_, status) = Self::state_label(&self.state(g.id));
                GroupRow {
                    id: g.id as i32,
                    name: g.name.clone().into(),
                    port: g.port.clone().into(),
                    status: status.into(),
                }
            })
            .collect::<Vec<_>>();
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
            let (reason, _) = Self::state_label(&state);
            if self.last_states.get(&id) != Some(&reason) {
                self.push(
                    id,
                    LogRow {
                        stamp: format!(
                            "[{}]",
                            core::local_hms(
                                core::epoch_ms(std::time::SystemTime::now()),
                                self.offset
                            )
                        )
                        .into(),
                        text: reason.clone().into(),
                        kind: if matches!(state, State::Waiting(_) | State::Error(_)) {
                            3
                        } else {
                            2
                        },
                    },
                );
                self.last_states.insert(id, reason.clone());
            }
            w.set_state_reason(reason.into());
            w.set_state_text(
                match &state {
                    State::Running => "运行中",
                    State::Waiting(_) => "等待重试",
                    State::Stopping => "停止中",
                    State::Stopped => "已停止",
                    State::Error(_) => "错误",
                }
                .into(),
            );
            w.set_state_led(match state {
                State::Running => 1,
                State::Waiting(_) | State::Error(_) => 2,
                _ => 0,
            });
            w.set_state_error(matches!(state, State::Waiting(_) | State::Error(_)));
            w.set_running_time(
                self.running_since
                    .get(&id)
                    .map(|since| {
                        let s = since.elapsed().as_secs();
                        format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
                    })
                    .unwrap_or_default()
                    .into(),
            );
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
            w.set_rx_count(
                stats
                    .map_or(0, |s| s.rx.load(Ordering::Relaxed))
                    .to_string()
                    .into(),
            );
            w.set_tx_count(
                stats
                    .map_or(0, |s| s.tx.load(Ordering::Relaxed))
                    .to_string()
                    .into(),
            );
            w.set_drop_count(
                stats
                    .map_or(0, |s| {
                        s.outputs
                            .iter()
                            .map(|(_, o)| o.dropped.load(Ordering::Relaxed))
                            .sum::<u64>()
                    })
                    .to_string()
                    .into(),
            );
            if let Some(s) = stats {
                if let Some(warning) = s.warning.lock().unwrap().as_ref() {
                    w.set_state_reason(format!("{}；{warning}", w.get_state_reason()).into());
                    w.set_state_error(true);
                }
            }
        } else {
            w.set_locked(false);
            w.set_stopping(false);
            w.set_desired_running(false);
            w.set_state_led(0);
            w.set_state_error(false);
            w.set_state_text("已停止".into());
            w.set_state_reason("请点击 ＋ 新建分组".into());
            w.set_running_time("".into());
            w.set_rx_count("0".into());
            w.set_tx_count("0".into());
            w.set_drop_count("0".into());
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
        self.sync_settings(w);
    }
    fn sync_settings(&self, w: &AppWindow) {
        if let Some(s) = &self.settings_window {
            s.global::<Theme>().set_dark(w.get_dark());
            s.set_theme_index(if w.get_dark() { 1 } else { 0 });
            s.set_close_actions(w.get_close_actions());
            s.set_close_index(w.get_close_index());
            s.set_autostart(w.get_autostart());
            s.set_pairs(w.get_pairs());
            s.set_has_unhidden(self.pairs.iter().any(|p| !p.b_hidden));
            s.set_pair_index(w.get_pair_index());
            s.set_message(w.get_manage_message());
            s.set_busy(w.get_manage_busy());
        }
    }
    pub fn settings_window(&self) -> Option<SettingsWindow> {
        self.settings_window
            .as_ref()
            .map(ComponentHandle::clone_strong)
    }
    fn start(&mut self, id: u32) {
        // A preview has no process instance and must not perform device IO.
        if self.instance.is_none() {
            self.errors.insert(id, "界面预览：不会打开串口".into());
            return;
        }
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
    pub fn initialize_view(&mut self, w: &AppWindow) {
        w.set_parities(strings(["None", "Odd", "Even"].map(str::to_string)));
        w.set_dark(self.settings.theme == core::Theme::Dark);
        w.global::<Theme>().set_dark(w.get_dark());
        w.set_show_data(self.settings.show_data);
        resize_window(w, self.settings.show_data);
        w.set_close_actions(strings(CloseAction::ALL.map(|c| c.label().to_string())));
        w.set_close_index(
            CloseAction::ALL
                .iter()
                .position(|c| *c == self.settings.close_action)
                .unwrap() as i32,
        );
        w.set_retention(self.settings.retention_days.min(3650) as i32);
        self.editor(w);
    }
    pub fn initialize(&mut self, w: &AppWindow) {
        self.initialize_view(w);
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
        if self.instance.as_ref().is_some_and(Instance::show_requested) {
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
        let hms = core::local_hms(event.ms, self.offset);
        let text = event.notice.unwrap_or_else(|| {
            if self.hex_mode {
                core::display_hex_line(&hms, event.direction, event.source.as_deref(), &event.raw)
            } else {
                core::display_text_line(&hms, event.direction, event.source.as_deref(), &event.raw)
            }
        });
        let text = if error {
            text
        } else {
            text.split_once("] ")
                .map_or(text.clone(), |(_, body)| body.to_string())
        };
        self.push(
            event.group,
            LogRow {
                stamp: format!("[{hms}]").into(),
                text: text.into(),
                kind: if error {
                    3
                } else if event.direction == core::Direction::Rx {
                    0
                } else {
                    1
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
                if let Some(s) = &self.settings_window {
                    let _ = s.hide();
                }
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

pub fn bind(w: &AppWindow, app: Rc<RefCell<App>>) -> Result<(), slint::PlatformError> {
    let settings = SettingsWindow::new()?;
    settings.set_app_icon(w.get_app_icon());
    bind_settings(w, &settings);
    app.borrow_mut().settings_window = Some(settings);
    let weak = w.as_weak();
    let a = app.clone();
    w.on_expand(move |show| {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            a.settings.show_data = show;
            a.save();
            w.set_show_data(show);
            resize_window(&w, show);
            w.invoke_scroll_end();
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_theme_setting(move |index| {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            a.settings.theme = if index == 1 {
                core::Theme::Dark
            } else {
                core::Theme::Light
            };
            a.save();
            w.set_dark(index == 1);
            w.global::<Theme>().set_dark(index == 1);
            a.sync_settings(&w);
        }
    });
    let a = app.clone();
    w.on_display_mode(move |hex| {
        a.borrow_mut().hex_mode = hex;
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_explain_state(move || {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            if let Some(id) = a.selected {
                a.push(
                    id,
                    LogRow {
                        stamp: "".into(),
                        text: w.get_state_reason(),
                        kind: if w.get_state_error() { 3 } else { 2 },
                    },
                );
            }
            a.settings.show_data = true;
            a.save();
            w.set_show_data(true);
            resize_window(&w, true);
            w.invoke_scroll_end();
        }
    });
    let weak = w.as_weak();
    w.on_rename_choice(move |save| {
        if let Some(w) = weak.upgrade() {
            if save && !w.get_locked() {
                let name = w.get_rename_text();
                if !name.trim().is_empty() {
                    w.invoke_edit(0, name);
                }
            }
            w.set_rename_dialog(false);
        }
    });
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
                a.history_widths.remove(&id);
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
                    if let Some(p) = match value.as_str() {
                        "None" => Some(Parity::None),
                        "Odd" => Some(Parity::Odd),
                        "Even" => Some(Parity::Even),
                        _ => Parity::ALL.into_iter().find(|p| p.label() == value),
                    } {
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
                a.history_widths.remove(&id);
                w.set_log_width(0.0);
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
            if a.instance.is_none() {
                w.set_autostart(winsys::autostart().unwrap_or(false));
                return;
            }
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
                "".into()
            });
            a.request_detect();
            a.sync_settings(&w);
            if let Some(s) = &a.settings_window {
                s.set_new_dialog(false);
                s.set_retention_text(a.settings.retention_days.to_string().into());
                s.set_new_port(w.get_new_port());
                let _ = s.show();
            }
        }
    });
    let weak = w.as_weak();
    let a = app.clone();
    w.on_manage(move |action| {
        if let Some(w) = weak.upgrade() {
            let mut a = a.borrow_mut();
            if a.instance.is_none() && action != 2 {
                w.set_manage_message("界面预览：不会调用 setupc".into());
                return;
            }
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
            } else if action == 3 {
                let indices: Vec<_> = a
                    .pairs
                    .iter()
                    .filter(|p| !p.b_hidden)
                    .map(|p| p.index)
                    .collect();
                if indices.is_empty() {
                    return;
                }
                c0c::Manage::HideBackends(indices)
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
                if let Some(s) = &a.settings_window {
                    let _ = s.hide();
                }
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
    Ok(())
}

fn resize_window(w: &AppWindow, expanded: bool) {
    let scale = w.window().scale_factor();
    // Physical sizing follows Slint's scale override, including deterministic preview DPI.
    w.window().set_size(slint::PhysicalSize::new(
        ((if expanded { 780.0 } else { 300.0 }) * scale).round() as u32,
        (334.0 * scale).round() as u32,
    ));
}

fn bind_settings(w: &AppWindow, s: &SettingsWindow) {
    let weak = w.as_weak();
    s.on_theme_setting(move |i| {
        if let Some(w) = weak.upgrade() {
            w.invoke_theme_setting(i);
        }
    });
    let weak = w.as_weak();
    s.on_close_setting(move |i| {
        if let Some(w) = weak.upgrade() {
            w.set_close_index(i);
            w.invoke_close_setting(i);
        }
    });
    let weak = w.as_weak();
    s.on_retention_input(move |text| {
        if let Some(w) = weak.upgrade() {
            if let Ok(days) = text.trim().parse::<u32>() {
                if days > 0 && days <= i32::MAX as u32 {
                    w.set_retention(days as i32);
                    w.invoke_retention_setting(days as i32);
                }
            }
        }
    });
    let weak = w.as_weak();
    s.on_startup(move |enabled| {
        if let Some(w) = weak.upgrade() {
            w.set_autostart(enabled);
            w.invoke_startup(enabled);
        }
    });
    let weak = w.as_weak();
    s.on_select_pair(move |i| {
        if let Some(w) = weak.upgrade() {
            w.set_pair_index(i);
        }
    });
    let weak = w.as_weak();
    let sw = s.as_weak();
    s.on_create_pair(move || {
        if let (Some(w), Some(s)) = (weak.upgrade(), sw.upgrade()) {
            s.set_new_port(w.get_new_port());
            s.set_new_dialog(true);
        }
    });
    let weak = w.as_weak();
    let sw = s.as_weak();
    s.on_manage(move |action| {
        if let (Some(w), Some(s)) = (weak.upgrade(), sw.upgrade()) {
            w.set_pair_index(s.get_pair_index());
            w.set_new_port(s.get_new_port());
            w.invoke_manage(action);
            s.set_message(w.get_manage_message());
            s.set_busy(w.get_manage_busy());
            if w.get_manage_busy() {
                s.set_new_dialog(false);
            }
        }
    });
    let weak = w.as_weak();
    s.on_open_data(move || {
        if let Some(w) = weak.upgrade() {
            w.invoke_open_data();
        }
    });
    let weak = w.as_weak();
    let sw = s.as_weak();
    s.on_done(move || {
        if let Some(s) = sw.upgrade() {
            let _ = s.hide();
        }
        if let Some(w) = weak.upgrade() {
            w.set_manage_dialog(false);
        }
    });
    let weak = w.as_weak();
    let sw = s.as_weak();
    s.window().on_close_requested(move || {
        if let Some(s) = sw.upgrade() {
            let _ = s.hide();
        }
        if let Some(w) = weak.upgrade() {
            w.set_manage_dialog(false);
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
