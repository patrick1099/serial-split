# 任务书：serial-split v1

给执行者（codex）的完整任务说明。用户已确认的需求见同目录 `HANDOFF.md`「已谈定」，本文件与它冲突时以它为准。

## 1. 这是什么

Windows 桌面小工具，纯 Rust + Slint。把一个**物理串口**拆成多个**虚拟串口**，让多个软件（协议模拟器、串口助手）同时用同一个物理口：

- 物理口收到的每个字节 → 原样发给该分组勾选的每一个虚拟口；
- 任一虚拟口上的软件发出的字节 → 写进物理口；
- 经过物理口的数据全部录进 SQLite。

虚拟口由 **com0com** 驱动提供（已决定，不自写驱动，不调用 hub4com）。com0com 每一对口互相连通：`CNCAn ⇄ CNCBn`。约定 **A 端给其他软件用**（改名为 COM10 这类），**B 端由本程序打开**（保持 `CNCBn` 名字，不出现在串口助手的下拉框里）。本程序从 B 端读到的就是其他软件发出的数据，写进 B 端的数据其他软件会从 A 端收到。

## 2. 功能（全部要做）

### 2.1 分组
- 一个分组 = 名称 + 物理口 + 波特率 / 数据位 / 校验 / 停止位 + 勾选的虚拟口（记 A 端名字，如 COM10）+ 是否录制。新建默认 2400 8N1、录制开。
- 可新建、删除（运行中不能删，按钮置灰）、改名，修改后立即保存到配置文件。
- 运行中的分组参数控件置灰，停了才能改。
- 一个虚拟口只能属于一个分组：别的分组已勾选的虚拟口显示为灰色并注明"被 <分组名> 使用"。
- 多个分组可同时运行；同一物理口不能被两个运行中的分组同时占（`core::validate_start` 已实现该检查）。
- 记住每个分组退出时是否在运行（`Group.running`）。启动后自动恢复这些分组；物理口打不开（没插、被占用）就显示原因，每 3 秒重试，直到成功或用户点停止。分组运行中物理口被拔掉/读写失败 → 同样转入"等待重试"状态。用户点"停止" → `running=false` 并保存；**退出程序不改 `running`**（下次开机要恢复）。

### 2.2 转发与录制
- 物理口按分组参数打开，流控 None。虚拟口（B 端）用任意波特率打开（com0com 默认不模拟波特率）。
- **线程模型（必须）**：serialport 4.10 在 Windows 上用同步句柄（见 serialport 4.10.1 源码 `src/windows/com.rs`：非 overlapped 的 `CreateFileW` + `ReadFile`/`WriteFile`），同一句柄的读和写会互相排队，所以**每个口只开一个线程、只有这个线程碰这个句柄**，在线程里交替做"短超时读"和"把待写队列写完"。不要用 `try_clone` 做一读一写双线程。
  - 物理口线程：读超时 10ms；读到数据 → 记录 RX、发到每个虚拟口线程的有界队列（`try_send`，满了就计入该虚拟口的丢弃字节数，不阻塞）。写之前用 `core::write_timeout(len, baud)` 调大超时，写完改回 10ms。写失败（非超时）视为物理口故障 → 该分组进入等待重试。
  - 虚拟口线程：读/写超时都用 20ms。读到数据 → 发进物理口线程的有界队列（满了就短暂等待并检查停止标志，**不丢**：这是要发给设备的命令）。往虚拟口写超时或写 0 字节（对端软件没打开或不读）→ 丢掉这块并计数，不算故障。
  - 物理口线程写进物理口成功后才记录 TX，`source` 填来源虚拟口的 A 端名字。
- 停止：置停止标志，**不在 UI 线程 join**；另起线程 join 完再通过消息告诉 UI"已停止"。停止过程中该分组的开始按钮置灰。
- 录制（分组勾选了"录制"才写）：单独一个录制线程持有 SQLite 连接，各口线程通过有界通道发消息（满了阻塞等待，录制数据不能静默丢）。每 200ms 或攒满 500 条提交一个事务。写库失败要在界面上醒目显示（顶部状态栏红字），转发不受影响。

### 2.3 SQLite
- 路径 `%LOCALAPPDATA%\serial-split\capture.sqlite3`，`PRAGMA journal_mode=WAL`（别的程序/AI 要能边录边只读查询）。
- 表结构：

```sql
CREATE TABLE IF NOT EXISTS sessions(
  id INTEGER PRIMARY KEY,
  group_name TEXT NOT NULL,
  port TEXT NOT NULL,
  line_settings TEXT NOT NULL,      -- 如 "2400 8E1"，用 Group::line_settings()
  outputs TEXT NOT NULL,            -- 如 "COM10,COM11"
  started_at TEXT NOT NULL,         -- core::utc_iso
  ended_at TEXT,
  end_reason TEXT                   -- 用户停止 / 程序退出 / 物理口故障: <原因> / 程序异常退出
);
CREATE TABLE IF NOT EXISTS chunks(
  id INTEGER PRIMARY KEY,
  session_id INTEGER NOT NULL REFERENCES sessions(id),
  utc TEXT NOT NULL,
  mono_us INTEGER NOT NULL,         -- 进程内单调时钟微秒，算同会话间隔用
  direction TEXT NOT NULL CHECK(direction IN ('RX','TX')),
  source TEXT,                      -- TX 的来源虚拟口（A 端名），RX 为 NULL
  raw BLOB NOT NULL
);
CREATE INDEX IF NOT EXISTS chunks_session ON chunks(session_id, id);
CREATE INDEX IF NOT EXISTS chunks_utc ON chunks(utc);
```

- 每次打开物理口成功开一个 session，关闭/故障时补 `ended_at`、`end_reason`。启动时把 `ended_at IS NULL` 的旧 session 补成"程序异常退出"（`ended_at` 取它最后一条 chunk 的 utc，没有就取 started_at）。
- 保留天数（全局设置，默认 30）：启动时和之后每 6 小时删一次过期 chunks 及已无 chunk 且已结束的过期 session。

### 2.4 窗口（Slint）
- 左栏：分组列表（名称、物理口、状态圆点：灰=停止、绿=运行、橙=等待重试、红=错误），下方"新建分组""删除分组"。
- 右侧：选中分组的编辑区——名称输入框；物理口下拉 + "刷新"；波特率下拉（`core::BAUDS`）；数据位（8/7）；校验（`Parity::ALL` 的 label）；停止位（1/2）；"转发到虚拟口"一行复选框（列出所有检测到的 com0com 对，显示 A 端名）；"录制到数据库"复选框；"开始/停止"按钮。
- 状态行：状态文字（运行中 / 等待重试：原因 / 已停止）+ 统计：物理口收、发字节数，每个虚拟口转出字节、丢弃字节（`core::human_bytes`）。
- 实时数据区：选中分组的最近数据，每行用 `core::display_line`（时间用 `core::local_hms`，时区偏移从 Windows `GetTimeZoneInformation` 取）。每个分组最多保留 2000 行（只是显示，丢最旧的）；"暂停显示""自动滚动""清屏"。RX 黑色、TX 蓝色、提示灰色、错误红色。等宽字体。
- 底栏（全局）：开机自启复选框；关闭窗口时（`CloseAction::ALL` 的 label）；数据保留天数；"打开数据目录"；"虚拟口管理…"。
- 虚拟口管理（弹层）：列出所有 com0com 对（序号、A 端名、B 端名、被哪个分组使用）；"新建一对"（输入框默认 `core::suggest_port_name`，用 `core::normalize_port_name` 校验）；"删除选中的一对"（被分组使用时拒绝）；"重新检测"。
- 中文显示：照此前一个 Slint 原型的做法，用 `slint::fontique_011` 注册 `C:\Windows\Fonts\msyh.ttc`，`default-font-family: "Microsoft YaHei"`；等宽区再注册 `consola.ttf`，用 "Consolas"。后端固定 winit + software 渲染（见 §4 依赖），参考：

```rust
slint::BackendSelector::new()
    .backend_name("winit".into())
    .renderer_name("software".into())
    .select()?;
// 注册字体：
let blob = slint::fontique_011::fontique::Blob::new(std::sync::Arc::new(bytes));
slint::fontique_011::shared_collection().register_fonts(blob, None);
```

### 2.5 托盘、关闭、自启、单实例
- 托盘（`tray-icon` 0.26）：图标用代码画的 32×32 RGBA（随意但可辨认，窗口图标用同一张）；左键单击/双击 → 显示窗口并置前；右键菜单："显示窗口""退出"。托盘提示文字显示运行中的分组数。
- 关闭窗口按 `CloseAction`：`Ask` → 窗口内弹层询问"最小化到托盘 / 退出程序 / 取消"，带"记住我的选择"复选框（勾了就把选择写回设置）；`Tray` → 隐藏窗口；`Exit` → 退出。窗口隐藏时事件循环必须继续（用 `slint::run_event_loop_until_quit()`，退出用 `slint::quit_event_loop()`）。
- 退出：停止全部分组（此时可以阻塞 join）、录制线程提交剩余数据并关库、保存设置。
- 开机自启：`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 下值 `serial-split` = `"<当前 exe 绝对路径>" --minimized`。界面复选框直接读写注册表（注册表是唯一真相，不存进 ini）。启动时若已开启但路径不是当前 exe，改成当前路径。
- `--minimized`：启动不显示窗口，只有托盘。
- 单实例：命名互斥量 `Local\serial-split.instance`；已存在 → 打开命名事件 `Local\serial-split.show` 并 `SetEvent`，然后自己退出。首个实例创建该事件（自动复位），UI 定时器里 `WaitForSingleObject(h, 0)` 检测到就显示窗口并置前。
- release 构建用 `#![windows_subsystem = "windows"]`；启动阶段致命错误用 `MessageBoxW` 弹出（没有控制台）。

### 2.6 com0com 检测与管理（`src/c0c.rs`，Windows 特定代码集中在这里）
- **开发机当时还没装 com0com**，下面的注册表位置你没法实机验证，所以检测逻辑必须集中在一个函数里，便于装好后修正：
  - 主路径：`HKLM\SYSTEM\CurrentControlSet\Services\com0com\Parameters\` 下的子键 `CNCA<n>` / `CNCB<n>`，值 `PortName`（没有该值时端口名就是子键名本身）。
  - 兜底：`HKLM\HARDWARE\DEVICEMAP\SERIALCOMM` 里值名形如 `\Device\com0com1<n>`（A 端）/ `\Device\com0com2<n>`（B 端），值数据是端口名。
  - 结果：`Vec<Pair { index, a_name, b_name }>`，按 index 排序。检测失败返回空列表，界面显示"没检测到 com0com 虚拟口对"，不崩。
- 物理口列表：`serialport::available_ports()`，去掉所有 com0com 端口名，按 `core::port_sort_key` 排序；蓝牙口在名字后标"（蓝牙）"。分组里保存的口当前不在时，下拉里仍显示为"COM6（未连接）"，不能把用户的选择冲掉。
- 测试开关 `--include-virtual`：物理口下拉里也列出 com0com 的 A 端（标"（虚拟）"），用于没有真实串口时拿一对虚拟口假扮物理口做联调。
- setupc 位置：`%ProgramFiles(x86)%\com0com\setupc.exe` 或 `%ProgramFiles%\com0com\setupc.exe`，找不到就在管理弹层提示"没装 com0com"。
- 新建：`setupc.exe --silent install PortName=<名字> -`；删除：`setupc.exe --silent remove <n>`。必须以 setupc 所在目录为工作目录，用 `ShellExecuteExW` + `lpVerb="runas"` + `SEE_MASK_NOCLOSEPROCESS` 提权运行（会弹 UAC），在后台线程等进程结束后通知 UI 重新检测；用户拒绝 UAC 要提示"已取消"。

### 2.7 配置文件
- `%APPDATA%\serial-split\settings.ini`，读写用 `core::parse_settings` / `core::format_settings`（已实现并有测试）。写入先写临时文件再重命名替换。

## 3. 代码结构

```
src/
  main.rs      入口：参数、单实例、路径、组装、事件循环、退出流程
  core.rs      纯规则（已存在，可扩展；不准引入 IO / 界面 / 注册表）
  bridge.rs    一个运行中分组：打开口、口线程、计数、停止
  recorder.rs  SQLite 录制线程
  c0c.rs       com0com 检测、物理口列表、setupc 提权调用
  winsys.rs    单实例、开机自启注册表、时区偏移、MessageBox
  tray.rs      托盘图标与菜单
  ui.rs        slint! 宏里的界面定义 + 界面状态与回调（UI 线程拥有控件）
examples/
  loopback.rs  联调脚本，见 §5
```

- 依赖单向：ui/main → bridge/recorder/c0c/winsys/tray → core。core 不认识任何外层。
- 后台线程不碰 Slint 对象；结果用有界 `std::sync::mpsc::sync_channel` 发给 UI，UI 用 `slint::Timer`（100ms）取消息、刷新计数。显示消息通道满了就丢显示消息并计数（只影响显示），不能阻塞口线程。
- 不引入 tokio / 异步运行时 / 线程池。
- 个人工具，代码里可以写注释，但只写必要的"为什么"。

## 4. 环境事实

- 工具链：`stable-x86_64-pc-windows-gnullvm`，rustc 1.99。链接器和 C 编译器来自 llvm-mingw，每条 cargo 命令前先把 llvm-mingw 的 `bin` 目录加到 PATH 最前面（否则报 `linker x86_64-w64-mingw32-clang not found`）。主聊已用这个 PATH 以空 main 编过整套依赖（dev 构建通过，`target/` 里有缓存）。
- `Cargo.toml` 的依赖已经加好、`cargo fetch` 已完成（slint =1.18.1 指定特性、serialport 4、winreg 0.55、tray-icon 0.26、rusqlite 0.40 bundled、windows-sys 0.61 带若干 feature）。需要别的 windows-sys feature 就自己加到已有那一行；**尽量不要再加新 crate**，确需新加先确认能离线解析，不行就在汇报里说明。
- 开发机当时没有 com0com，也没有插 USB 转串口，只有蓝牙串口（**不要打开蓝牙串口**，会触发蓝牙连接）。实机联调由主聊在装好 com0com 后做。

## 5. 完成标准

1. `cargo test` 全过（`core.rs` 现有测试不许删改其断言；新增的纯逻辑也要有测试）。
2. `cargo build --release` 通过，无 warning 或说明剩余 warning 的原因。
3. 启动 release exe 能出窗口、托盘图标出现、没有 com0com 时界面给出提示而不崩；能新建分组、改参数、重启后配置还在；关闭弹层三个选项都能走通；`--minimized` 不出窗口；第二次启动会唤出第一个实例。能自己验证的就验证，验证不了的在汇报里列出来。
4. `examples/loopback.rs`：参数 `<假物理口的B端> <虚拟口A端...>`（如 `CNCB2 COM10 COM11`）。它打开这些口，向假物理口的 B 端写一串带编号的数据，检查每个虚拟口 A 端都原样收到；再从每个虚拟口 A 端各发一串，检查假物理口 B 端都收到（顺序可交错，但每串内部完整）。打印逐项 PASS/FAIL，有 FAIL 就非零退出。你写好、能编译即可，运行由主聊做。
5. 在仓库根建 `AGENTS.md`，只放一段：

```markdown
## 架构约束（vibe-apps）
实现选择：原生桌面，纯 Rust + Slint 1.18（winit + software 渲染），Windows 单 exe 交付；虚拟口依赖 com0com 驱动。
业务规则与 UI/IO 分离，可独立测试；应用状态由明确的所有者维护。
每个串口只由一个线程持有并读写；后台线程不碰 Slint 对象，结果经有界通道回 UI 定时器；录制数据不得静默丢弃，显示数据可限量。
```

## 6. 边界

- 只在本仓库的开发工作树里改文件。
- **不要 git commit**，改动留在工作区，主聊审完再提交。
- 不要写 BLUEPRINT / CHANGELOG / README，不要改 `docs/HANDOFF.md` 和本文件。
- 不要运行任何需要管理员权限的命令，不要真的去调用 setupc。
- 汇报：改了哪些文件；`cargo test` / `cargo build --release` 的结果原文摘要；§5 第 3 条每项验证了没有；你对哪些地方没把握（特别是 com0com 注册表位置、托盘与 winit 事件循环的配合）。
