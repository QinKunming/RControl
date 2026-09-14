# RControl

**Windows 轻量级远程控制软件（直连版）** —— Rust 实现，单文件免依赖，端到端 AES-256-GCM 加密。

主控端直连被控端（IP:端口），数据不经过任何第三方服务器。被控端支持
Windows 7 / Windows Server 2019 / Win10 / Win11，主控端支持 Windows 7 及以上。

> 需要内网穿透（被控端在 NAT 后）？请看姊妹项目
> **[RControlRelay](https://github.com/qinkunming/RControlRelay)**（中继版，本项目的增强版本）。

## 功能特性

- **远程画面**：GDI 截屏 + JPEG 压缩，按被控端原始分辨率 **1:1 像素显示**（两边点击位置严格一致），
  4K 支持良好；可调画质/帧率/最大宽度以适配带宽
- **键鼠控制**：SendInput 注入，支持仅查看模式
- **文件传输**：双栏资源管理器（远程/本地），浏览/新建/删除/重命名/上传/下载，
  大文件分块流式传输，实时进度与速度
- **剪贴板**：文本双向自动同步，最大 4MB
- **锁屏下可登录**：服务模式运行时，远程即可见锁屏界面，发送 Ctrl+Alt+Del 后
  直接输入 Windows 登录密码远程登录（被控端无人值守开机也能进系统）
- **服务模式**：SCM 服务开机自启，用户注销/会话切换自动重连，自动添加防火墙规则
- **被控端清单**：多台被控端参数保存在 `hosts.txt`，双击即连
- **自动化自测**：`--selftest` 一条命令完成 11 项端到端检查（认证/画面/键鼠/文件/剪贴板）
- **小体积**：被控端 1.7MB（Win7 版 2.1MB）/ 主控端 1.2~1.6MB，**无任何运行时依赖**

## 架构设计

```
┌──────────────┐                        ┌──────────────┐
│  rcontrol_   │   TCP 直连（可跨公网）   │  rcontrol_   │
│   client     │ ◄────────────────────► │   server     │
│  主控端 GUI   │   端到端 AES-256-GCM    │  被控端服务   │
└──────────────┘                        └──────────────┘
```

| crate | 职责 |
|---|---|
| `rcontrol_common` | 协议与编解码：protobuf 消息定义、帧格式、加密握手 |
| `rcontrol_server` | 被控端：SCM 服务 + 工作进程（截屏/输入/文件/剪贴板）|
| `rcontrol_client` | 主控端：Win32 GUI（清单管理、远程画面、文件传输）|

**被控端进程模型（rustdesk 式）**

- SCM 服务（SYSTEM，session 0）仅做看门狗：复制 `winlogon.exe` 令牌，用
  `CreateProcessAsUserW` 把 `--worker` 工作进程拉进活动控制台会话；会话切换或
  worker 被杀自动重拉
- worker 负责全部实际工作：GDI 截屏（可切换 winlogon 安全桌面，锁屏可见）、
  SendInput 键鼠注入、SendSAS（Ctrl+Alt+Del）、文件传输、剪贴板

**通信协议**

- 帧格式：4 字节大端长度前缀 + protobuf 消息体
- 安全通道：明文握手交换 nonce → HKDF 派生 AES-256-GCM 会话密钥（每次连接不同）
  → 双向口令认证 → 全程加密

## 安全机制

- 端到端 AES-256-GCM 加密，密钥由口令 + 双方随机数派生
- 双向认证：口令错误无法连接，且能防服务端伪造
- 同一口令连续错 5 次，来源 IP 封禁 60 秒
- 同一时间只接受一个主控连接
- 纯直连：数据不经过任何第三方服务器

## 编译构建

依赖：Rust（stable-msvc 工具链）+ Visual Studio Build Tools。

```
cargo build --release
```

产物：`target/release/rcontrol_server.exe`（被控端）、`target/release/rcontrol_client.exe`（主控端）。

**Win7 兼容版**：Win7 上官方支持的最后一个 Rust 系列是 1.77，用
`x86_64-pc-windows-gnu` 工具链（1.77.2）静态链接 CRT 单独编译：

```
rustup toolchain install 1.77.2 --profile minimal
cargo +1.77.2 build --release -p rcontrol_client --target-dir target_win7
cargo +1.77.2 build --release -p rcontrol_server --target-dir target_win7
```

Cargo.lock 已钉好 1.77 可解析的依赖版本；GNU 工具链无 windres 时
build.rs 自动回退 Windows SDK 的 rc.exe + cvtres.exe 编译资源。

## 快速上手

**被控端**（以管理员身份运行 `rcontrol_server.exe`）：

1. 首次运行自动生成访问口令，监听端口默认 `3333`
2. 点"安装服务"→"启动服务"（推荐，锁屏也能被控制）
3. 记下 GUI 显示的 口令 和本机 IP

**主控端**（运行 `rcontrol_client.exe`）：

1. 在被控端清单里双击一台（或填 地址/口令 点"连接"）
2. 窗口内即远程桌面，鼠标键盘直接操作

| 快捷键 | 功能 |
|---|---|
| `Esc` | 断开连接 |
| `F9` | 打开文件传输窗口 |
| `Ctrl+Shift+Del` | 向被控端发送 Ctrl+Alt+Del（锁屏登录用）|

**自动化自测**：

```
rcontrol_client.exe --selftest IP:端口 口令     # 11 项检查，退出码 0 = 全过
rcontrol_client.exe --view IP:端口 口令         # 跳过对话框直连
```

## 仓库结构

```
rcontrol_v1/
├── rcontrol_common/     # 协议与编解码
├── rcontrol_server/     # 被控端
├── rcontrol_client/     # 主控端（Win32 GUI）
├── shots/               # 构建/测试/截图验证脚本（PowerShell）
└── README.md / LICENSE
```

## 版本

- **v1.0**（2026-09）：首个开源版本，功能如上
- 版本分支独立维护：`v1.0` 分支冻结本期内容，`main` 跟随最新版本

## 声明

本项目仅供**合法授权**的远程管理与技术学习使用——请仅在你拥有或已获得明确授权的
设备上部署使用，遵守当地法律法规。使用者须自行承担不当使用的责任。

## 许可证

[MIT](LICENSE)
