## 架构约束（vibe-apps）
实现选择：原生桌面，纯 Rust + Slint 1.18（winit + software 渲染），Windows 单 exe 交付；虚拟口依赖 com0com 驱动。
业务规则与 UI/IO 分离，可独立测试；应用状态由明确的所有者维护。
每个串口只由一个线程持有并读写；后台线程不碰 Slint 对象，结果经有界通道回 UI 定时器；录制数据不得静默丢弃，显示数据可限量。
