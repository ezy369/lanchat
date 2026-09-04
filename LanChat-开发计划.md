# LanChat 开发计划

## 项目概述

LanChat 是一个用 Rust + GPUI 重写的飞秋（FeiQ）局域网即时通讯工具，定位为高质量开源替代品。项目采用多 crate workspace 架构，先兼容 IP Messenger 协议，后续渐进迁移到现代协议。

## 技术决策

| 决策项 | 选择 | 理由 |
|--------|------|------|
| 定位 | 高质量开源替代品 | 既有实际价值，又能深入练习 Rust + GPUI |
| 协议 | 先兼容 IPMsg，后渐进迁移 | 能和现有飞秋互通，降低推广阻力 |
| 平台 | Windows + macOS | GPUI 支持最成熟的两个平台 |
| UI 风格 | 简约现代 | 匹配"UI 太老旧"的改进诉求，GPUI 易实现 |
| 窗口模型 | 单窗口 + 侧边栏 | 最现代的体验，GPUI 天然适配 |
| 存储 | libSQL 嵌入式 | 去中心化，兼容 SQLite，支持 FTS5 全文搜索。曾评估 Turso（纯 Rust SQLite 重写），但其不支持 FTS5（`no such module: fts5`，MATCH/rank/snippet 均缺失），且构建需 Windows SDK 的 rc.exe，故保留 libSQL |
| 异步运行时 | Tokio | Rust 生态事实标准 |
| 日志 | tracing | 结构化日志，异步友好 |
| 序列化 | serde | Rust 序列化标准 |

## V1 功能范围

**核心功能（必须）：**
- 局域网自动发现（UDP 广播，无需配置）
- 一对一文本聊天
- 文件/文件夹发送（拖拽发送）
- 在线状态显示（在线/离开/忙碌）
- 用户列表（自动扫描网段内在线用户）
- 聊天记录本地保存与搜索（libSQL）

**V1 不包含（后续版本）：**
- 群聊/群组消息
- 消息提醒音
- 截图发送
- 消息加密
- 自定义头像/昵称
- 系统托盘 + 气泡通知
- 多语言支持
- 皮肤/主题切换

## 项目架构

```
fly-q-rs/
├── Cargo.toml                 # workspace 根配置
├── crates/
│   ├── flyq-protocol/         # IPMsg 协议解析与构建
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── command.rs     # 命令码定义
│   │       ├── packet.rs      # 数据包解析/构建
│   │       └── types.rs       # 协议类型定义
│   │
│   ├── flyq-network/          # 局域网发现与通信
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── discovery.rs   # UDP 广播发现
│   │       ├── transport.rs   # TCP 消息收发
│   │       └── peer.rs        # 节点管理
│   │
│   ├── flyq-storage/          # 数据存储层
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── db.rs          # libSQL 连接管理
│   │       ├── models.rs      # 数据模型
│   │       └── queries.rs     # SQL 查询
│   │
│   ├── flyq-ui/               # GPUI 界面层
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── app.rs         # 主窗口
│   │       ├── sidebar.rs     # 用户列表侧边栏
│   │       ├── chat.rs        # 聊天区域
│   │       └── components/    # 可复用 UI 组件
│   │
│   └── flyq-core/             # 业务逻辑胶水层
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── app_state.rs   # 应用状态管理
│           └── handlers.rs    # 事件处理器
│
└── src/
    └── main.rs                # 程序入口
```

## 里程碑计划

### M1：骨架搭建（约 1 周）

**目标：** workspace 搭建完成，协议解析库和存储层基本可用。

**交付物：**
- [ ] Cargo workspace 配置完成，各 crate 依赖关系确立
- [ ] `flyq-protocol`：IPMsg 协议解析库，支持核心命令（上线通知、下线通知、发送消息、接收消息）
- [ ] `flyq-storage`：libSQL 存储层，消息和用户的 CRUD 操作
- [ ] 单元测试覆盖协议解析和存储操作

**验收标准：** 协议解析单测全绿，存储层能写入/读取消息。

### M2：网络层 + 局域网发现（约 1-2 周）

**目标：** 两个实例能在局域网内互相发现。

**交付物：**
- [ ] `flyq-network`：UDP 广播发现机制
- [ ] `flyq-network`：TCP 消息收发
- [ ] `flyq-network`：用户上下线感知
- [ ] 集成测试：两个实例互相发现

**验收标准：** 两个实例能在局域网内互相发现并出现在用户列表中。

### M3：UI + 聊天功能（约 2-3 周）

**目标：** 两个人能用 LanChat 互相发消息。

**交付物：**
- [ ] `flyq-ui`：主窗口布局（侧边栏 + 聊天区）
- [ ] `flyq-ui`：用户列表显示
- [ ] `flyq-ui`：聊天界面（消息输入、消息列表）
- [ ] `flyq-core`：消息收发与持久化集成
- [ ] 端到端测试：两人互发消息

**验收标准：** 两个人能用 LanChat 互相发消息，关闭重开能看到历史记录。

### M4：文件传输 + 打磨（约 1-2 周）

**目标：** 文件传输可用，v1 发布。

**交付物：**
- [ ] 文件/文件夹发送与接收
- [ ] 传输进度显示
- [ ] 基础系统托盘
- [ ] 基础通知
- [ ] README 文档
- [ ] 发布包（Windows .msi / macOS .dmg）

**验收标准：** 能成功发送文件给对方，对方能接收保存。

> **实际进度：** 单文件传输与传输进度显示已完成（提交 `a8bc711`）；文件夹传输、基础系统托盘、基础通知、README、发布包尚未交付，并入下方 M5。

### M5：v1 收尾 + 体验完善（约 2-3 周）

**目标：** 补齐 M4 未交付的 v1 发布项，并完善配置与在线状态体验，产出可安装发布的 v1。

**范围说明：** 群聊/群组消息涉及组播语义与多端会话状态，体量与风险最大，单独列为 M6，不在 M5 范围内。

**交付物：**

*文件夹传输（补齐 M4 遗留）*
- [x] 目录递归传输后端：`FileRegistry` 递归登记目录、`Transport` 逐文件收发并重建目录树、`handlers` 聚合进度（防目录穿越），补测试
- [x] 文件夹传输 UI：目录选择器（`prompt_for_paths{directories:true}`）+ 文件夹拖拽 + "x/y 文件 + 百分比"聚合进度

*配置与状态*
- [x] 配置持久化：`Config`（昵称/下载目录/端口）落盘用户目录，启动加载、变更保存
- [x] 设置面板：gpui-component 表单，可视化编辑昵称/下载目录/端口
- [x] 在线状态 UI：设置自身"在线/离开/忙碌"并经 presence 报文广播、写入配置（接收/着色链路已就绪）

*桌面集成（需先做可行性验证）*
- [ ] 基础系统托盘：GPUI 无一等支持，验证 `tray-icon` 可行性后接线（显示主窗口 / 退出）
- [ ] 基础通知：`notify-rust`，新消息 / 文件到达 / 传输完成时触发

*发布*
- [ ] README 文档：简介 / 特性 / 截图 / 构建 / 使用 / 协议兼容性 / 架构
- [ ] 发布包：Windows `.msi`（cargo-wix / WiX）、macOS `.dmg`（cargo-bundle）

**验收标准：** 能发送并接收整个文件夹；可在设置面板修改昵称/下载目录/端口并持久化生效；能设置自身在线状态并被对端正确显示；系统托盘与通知可用；产出可安装运行的发布包，README 完整。

**风险与前置：** 系统托盘在 GPUI 生态无现成方案，可能引入新依赖并需先做可行性验证；发布包工具链（WiX / cargo-bundle）需额外安装；当前构建环境缺 Windows SDK 的 `rc.exe`，若打包需编译 Windows 资源须先补齐该环境。

## 参考项目

| 用途 | 项目 | 说明 |
|------|------|------|
| 飞秋协议参考 | [compilelife/feiq](https://github.com/compilelife/feiq) | C++/Qt，242 stars，飞秋扩展协议最完整 |
| IPMsg 协议实现 | [blisssayyid/feiX](https://github.com/blisssayyid/feiX) | Electron，37 stars，协议实现清晰 |
| 现代 LAN 通讯 UX | [localsend/localsend](https://github.com/localsend/localsend) | Flutter，90k stars，UX 标杆 |
| 全功能 LAN Messenger | [hasankhan/Squiggle](https://github.com/hasankhan/Squiggle) | .NET/Avalonia，127 stars |
| Rust LAN 聊天 | [v0l/lan_chat](https://github.com/v0l/lan_chat) | Rust/egui，唯一 Rust LAN 聊天项目 |
| UI 框架 | [longbridge/gpui-component](https://github.com/longbridge/gpui-component) | Rust/GPUI，13.7k stars |
| GPUI 生态 | [zed-industries/awesome-gpui](https://github.com/zed-industries/awesome-gpui) | GPUI 项目列表 |

## 技术栈

- **语言：** Rust
- **UI 框架：** GPUI + gpui-component
- **异步运行时：** Tokio
- **协议：** IP Messenger（UDP 2425 + TCP 2425）
- **存储：** libSQL（嵌入式）
- **日志：** tracing
- **序列化：** serde + serde_json
- **构建工具：** Cargo workspace
