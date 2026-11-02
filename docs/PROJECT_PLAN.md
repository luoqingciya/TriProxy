<!--
  TriProxy — a modern multi-core proxy client.
  Copyright (C) 2026 luoqingci
  SPDX-License-Identifier: GPL-3.0-or-later

  This program is free software: you can redistribute it and/or modify it under
  the terms of the GNU General Public License as published by the Free Software
  Foundation, either version 3 of the License, or (at your option) any later
  version. It is distributed in the hope that it will be useful, but WITHOUT ANY
  WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
  PARTICULAR PURPOSE. See LICENSE for the full text.

  Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.
-->

# 三内核现代化代理客户端完整项目规划

**项目暂定代号：TriProxy**

> 一个基于 Rust 构建的现代化跨平台代理客户端，统一管理 Mihomo、sing-box 和 Xray 三个核心，通过智能兼容性分析自动选择最合适的核心，并提供现代化、高性能、易用的桌面界面。

> **本文是规划书，不是现状。** 它描述的是目标态与当时的取舍，落地过程中的偏差、补充的决策
> 与当前实现范围都在 [`ARCHITECTURE.md`](ARCHITECTURE.md)；两者的差异集中在架构文档第 2 章
> 「与规划书的差异」。看「现在能做什么」请看 README，看「为什么这样做」看架构文档。

---

# 1. 项目背景与定位

目前代理客户端生态中存在两个明显问题：

1. 功能强大的传统客户端通常界面较为陈旧、复杂。
2. 现代化客户端通常只重点支持一个或少数几个核心。

本项目希望解决的问题是：

> 让用户不需要理解 Mihomo、sing-box、Xray 之间的复杂区别，也能获得较好的兼容性和现代化使用体验。

项目核心理念：

```text
用户只需要：

导入配置
    ↓
选择节点
    ↓
连接

剩下的事情：

配置解析
协议识别
能力分析
核心选择
配置转换
网络管理

全部由程序完成
```

---

# 2. 产品目标

## 2.1 核心目标

支持三个代理核心：

- Mihomo
- sing-box
- Xray

提供：

- 统一配置管理
- 多种订阅格式导入
- 自动核心选择
- 手动核心选择
- 节点管理
- 延迟测试
- 规则分流
- 系统代理
- TUN 模式
- DNS 管理
- 流量监控
- 连接管理
- 日志查看
- 自动更新

## 2.2 核心竞争力

### ⚡ Smart Engine

智能分析配置和节点能力，自动选择最合适的代理核心。

### 🎨 Modern UI

现代化、高质量、流畅的桌面客户端界面。

### 🧠 Unified Experience

不同配置格式、不同协议、不同核心统一成一致的用户体验。

---

# 3. 产品原则

## 3.1 用户不应该被迫理解 Core

普通用户默认看到：

```text
Smart Mode ⚡

Automatically optimized
```

高级设置中提供：

```text
Core Engine

● Smart (Recommended)
○ Mihomo
○ sing-box
○ Xray
```

## 3.2 内核默认不同时运行

任何时刻默认只允许一个 Active Core：

```text
Mihomo     Stopped
sing-box   Running
Xray       Stopped
```

避免多个核心同时创建 TUN、修改 DNS、修改路由、设置系统代理或占用相同端口。

## 3.3 UI 与 Core 完全解耦

推荐架构：

```text
React UI
    ↓
Tauri Commands
    ↓
Rust Application Layer
    ↓
Unified API
    ↓
Core Adapter
    ↓
Mihomo / sing-box / Xray
```

---

# 4. 总体架构

```text
┌──────────────────────────────────────────────┐
│                Frontend UI                   │
│         React + TypeScript + Tauri           │
├──────────────────────────────────────────────┤
│              Application Layer               │
│ Profile / Subscription / Core / Network      │
├──────────────────────────────────────────────┤
│                Unified API                   │
├───────────────────┬───────────────┬──────────┤
│  Mihomo Adapter   │ Sing-box      │ Xray     │
│                   │ Adapter       │ Adapter  │
├───────────────────┴───────────────┴──────────┤
│              Core Processes                  │
│ Mihomo Binary / sing-box Binary / Xray       │
├──────────────────────────────────────────────┤
│          Operating System Layer              │
│ System Proxy / TUN / DNS / Route             │
└──────────────────────────────────────────────┘
```

---

# 5. 技术栈

## 后端

- Rust
- Tokio
- Serde
- Reqwest
- Tracing
- Anyhow / Thiserror
- SQLite（SQLx 或 SeaORM）

职责：

- Core 生命周期管理
- 配置解析
- 配置转换
- 网络管理
- 订阅更新
- 数据存储
- IPC/API

## 桌面框架

推荐：

```text
Tauri
```

## 前端

推荐：

- React
- TypeScript
- Tailwind CSS
- Zustand
- React Router
- Motion

---

# 6. 项目目录

```text
tri-proxy/
├── README.md
├── LICENSE
├── docs/
│   ├── PROJECT_PLAN.md
│   ├── ARCHITECTURE.md
│   ├── CORE_SUPPORT.md
│   ├── CONFIG_MODEL.md
│   └── UI_DESIGN.md
├── src/
│   ├── pages/
│   │   ├── Dashboard/
│   │   ├── Proxies/
│   │   ├── Profiles/
│   │   ├── Rules/
│   │   ├── Connections/
│   │   ├── Logs/
│   │   └── Settings/
│   ├── components/
│   ├── stores/
│   ├── hooks/
│   ├── services/
│   └── types/
├── src-tauri/
│   ├── src/
│   │   ├── main.rs
│   │   ├── app/
│   │   ├── core/
│   │   │   ├── trait.rs
│   │   │   ├── manager.rs
│   │   │   ├── selector.rs
│   │   │   ├── capability.rs
│   │   │   └── health.rs
│   │   ├── cores/
│   │   │   ├── mihomo/
│   │   │   ├── singbox/
│   │   │   └── xray/
│   │   ├── profile/
│   │   ├── subscription/
│   │   ├── parser/
│   │   ├── network/
│   │   ├── storage/
│   │   └── commands/
│   ├── binaries/
│   │   ├── mihomo/
│   │   ├── sing-box/
│   │   └── xray/
│   └── Cargo.toml
└── package.json
```

---

# 7. Unified Profile

内部禁止直接让业务层依赖：

- `mihomo.yaml`
- `sing-box.json`
- `xray.json`

统一流程：

```text
输入配置
    ↓
Parser
    ↓
Unified Profile
    ↓
Core Compiler
    ↓
Core Config
```

概念模型：

```text
Profile
├── Metadata
├── Nodes
├── Groups
├── Rules
├── DNS
└── Network
```

示例：

```rust
pub struct Profile {
    pub id: String,
    pub name: String,
    pub metadata: ProfileMetadata,
    pub nodes: Vec<Node>,
    pub groups: Vec<ProxyGroup>,
    pub rules: Vec<Rule>,
    pub dns: Option<DnsConfig>,
    pub network: NetworkConfig,
}
```

---

# 8. 节点模型

第一阶段支持：

- Shadowsocks
- VMess
- VLESS
- Trojan
- WireGuard

后续扩展：

- Hysteria
- TUIC
- SOCKS
- HTTP
- SSH

统一模型：

```rust
pub enum Node {
    Shadowsocks(ShadowsocksNode),
    Vmess(VmessNode),
    Vless(VlessNode),
    Trojan(TrojanNode),
    WireGuard(WireGuardNode),
}
```

---

# 9. Core 抽象接口

三个核心实现统一 Trait：

```rust
#[async_trait]
pub trait ProxyCore: Send + Sync {
    fn name(&self) -> &str;

    fn version(&self) -> String;

    fn capability(&self) -> CoreCapability;

    async fn check_config(
        &self,
        config_path: &Path,
    ) -> Result<()>;

    async fn start(
        &mut self,
        config_path: &Path,
    ) -> Result<()>;

    async fn stop(&mut self) -> Result<()>;

    async fn restart(
        &mut self,
        config_path: &Path,
    ) -> Result<()>;

    async fn status(&self) -> CoreState;

    async fn health_check(&self) -> Result<CoreHealth>;
}
```

---

# 10. 三核心支持规划

## Mihomo

重点：

- Clash YAML
- Proxy Groups
- Rules
- Rule Providers
- Controller API
- System Proxy
- TUN
- DNS

定位：

> Clash 配置和订阅生态兼容。

## sing-box

重点：

- 现代协议
- TUN
- DNS
- Rule Sets
- 复杂路由
- 多协议支持

定位：

> 通用现代核心。

## Xray

重点：

- VMess
- VLESS
- Reality
- Vision
- Trojan
- 复杂 Transport

定位：

> V2Ray/Xray 原生生态兼容。

---

# 11. Smart Core Engine

提供：

```text
● Smart Mode
○ Mihomo
○ sing-box
○ Xray
```

默认：

```text
Smart Mode
```

自动选择流程：

```text
用户导入 Profile
        ↓
Profile Analyzer
        ↓
Feature Detection
        ↓
Compatibility Engine
        ↓
Core Score Engine
        ↓
Best Core
```

---

# 12. Capability System

Core 能力：

```rust
pub struct CoreCapability {
    pub protocols: Vec<Protocol>,
    pub supports_tun: bool,
    pub supports_dns: bool,
    pub supports_reality: bool,
    pub supports_vision: bool,
    pub supports_wireguard: bool,
    pub supports_rule_set: bool,
    pub supports_proxy_groups: bool,
}
```

Profile 需求：

```rust
pub struct ProfileRequirement {
    pub protocols: Vec<Protocol>,
    pub requires_tun: bool,
    pub requires_reality: bool,
    pub requires_vision: bool,
    pub requires_wireguard: bool,
}
```

---

# 13. 自动选择算法

第一阶段采用：

```text
Step 1：硬性兼容性检查
Step 2：淘汰不支持必须功能的 Core
Step 3：评分
Step 4：选择最高分
```

建议评分：

```text
Compatibility     50%
Feature Match     25%
Performance       15%
User Preference   10%
```

原则：

> 硬性兼容性不满足时直接淘汰，不能仅依赖综合评分。

---

# 14. Core 切换流程

完整流程：

```text
1. Lock CoreManager
2. Stop Current Core
3. Wait Process Exit
4. Cleanup System Proxy
5. Cleanup DNS
6. Cleanup TUN
7. Cleanup Route
8. Generate New Config
9. Validate Config
10. Start New Core
11. Health Check
12. Enable Network Features
13. Unlock CoreManager
```

状态机：

```text
Stopped
   ↓
Starting
   ↓
Running
   ↓
Stopping
   ↓
Stopped

异常：

Starting → Failed
```

---

# 15. Config Compiler

每个 Core 使用独立 Compiler：

```text
Unified Profile
       │
       ├─────────────┐
       ▼             ▼
MihomoCompiler  SingBoxCompiler
       │             │
       ▼             ▼
config.yaml     config.json

                     │
                     ▼
                XrayCompiler
                     │
                     ▼
                  xray.json
```

接口：

```rust
pub trait ConfigCompiler {
    fn compile(
        &self,
        profile: &Profile,
    ) -> Result<CompiledConfig>;
}
```

---

# 16. Network Manager

```text
NetworkManager
├── SystemProxyManager
├── TunManager
├── DNSManager
└── RouteManager
```

职责：

## System Proxy

- Enable
- Disable
- Restore Previous Settings

## TUN

- Permission Check
- Enable
- Disable
- Route Cleanup

## DNS

- Apply
- Restore
- Leak Protection

---

# 17. 订阅系统

支持：

- Subscription URL
- Clash YAML
- Base64
- URI List
- Local File

节点 URI：

- `vmess://`
- `vless://`
- `trojan://`
- `ss://`

后续：

- `hysteria2://`
- `tuic://`

生命周期：

```text
Subscription URL
        ↓
Downloader
        ↓
Parser
        ↓
Unified Profile
        ↓
Validate / Merge
        ↓
Database
```

支持：

- Manual Update
- Auto Update
- Update Interval
- Last Update
- Error State

---

# 18. 数据存储

推荐：

```text
SQLite
```

保存：

- Profiles
- Subscriptions
- Settings
- Favorites
- Node History
- Core Preferences

敏感数据优先使用系统安全存储：

- Windows Credential Manager
- macOS Keychain
- Linux Secret Service

原则：

> 不明文保存敏感凭据。

---

# 19. UI 设计原则

视觉目标：

```text
Modern
Minimal
Smooth
Fast
Clear
```

设计参考方向：

- FLClash 的现代化代理客户端体验
- Raycast 的效率感
- Arc 的简洁感
- Linear 的信息层级

原则：

> 功能可以复杂，但界面不能复杂。

---

# 20. Design System

统一定义：

- Color
- Typography
- Spacing
- Radius
- Shadow
- Animation

推荐间距：

```text
4
8
12
16
24
32
48
```

圆角：

```text
8px
12px
16px
```

主题：

- Light
- Dark
- System

---

# 21. 页面规划

```text
Dashboard
Proxies
Profiles
Rules
Connections
Logs
Settings
```

---

# 22. Dashboard

Dashboard 回答用户：

1. 当前是否连接？
2. 当前节点？
3. 当前延迟？
4. 当前速度？
5. 当前 Core？
6. 当前网络模式？

示例：

```text
┌──────────────────────────────────────────┐
│              Connected                   │
│                  ●                       │
│                                          │
│       Singapore Premium                  │
│                  38 ms                   │
├──────────────────────────────────────────┤
│ Download                Upload            │
│ 24.8 MB/s               3.2 MB/s          │
├──────────────────────────────────────────┤
│ ⚡ Smart Engine                           │
│ sing-box                                 │
│ Automatically Selected                    │
│ [ Why this Core? ]                       │
└──────────────────────────────────────────┘
```

---

# 23. Smart Engine 可解释性

用户点击：

```text
Why this Core?
```

显示：

```text
Smart Engine Analysis

Current Profile:
✓ VLESS
✓ Reality
✓ TUN
✓ DNS

Mihomo
Compatibility: 90%

sing-box
Compatibility: 100%
Selected

Xray
Compatibility: 100%

Reason:
Best feature compatibility
TUN supported
DNS supported
Current configuration optimized
```

原则：

> 自动决策必须可解释。

---

# 24. Proxies 页面

主要功能：

- Search
- Filter
- Sort
- Favorites
- Latency Test
- Batch Test

筛选：

- 按地区
- 按协议
- 按延迟
- 按收藏
- 搜索节点名称

建议使用虚拟列表支持大量节点。

---

# 25. Profiles 页面

展示：

```text
┌──────────────────────────┐
│ ☁ Subscription A         │
│                          │
│ 42 Nodes                 │
│ Updated 10 min ago       │
│                          │
│ ● Active                 │
└──────────────────────────┘
```

支持：

- Add
- Import
- Delete
- Update
- Duplicate
- Export

---

# 26. Connections 页面

显示：

- Application
- Destination
- Protocol
- Upload
- Download
- Rule
- Outbound

支持：

- Search
- Close Connection
- Auto Refresh

---

# 27. Logs 页面

支持：

- Core Logs
- Application Logs
- Network Logs

功能：

- Search
- Filter
- Copy
- Export
- Auto Scroll

日志等级：

- Debug
- Info
- Warn
- Error

---

# 28. Settings 页面

采用 Progressive Disclosure。

## General

- Start at Login
- Auto Connect
- Language
- Theme

## Network

- System Proxy
- TUN
- DNS
- Routing

## Core

- Smart Mode
- Mihomo
- sing-box
- Xray

## Advanced

- API
- Log Level
- Experimental Features

---

# 29. 前后端通信

```text
Frontend
    ↓
Tauri invoke()
    ↓
Rust Commands
    ↓
Application Services
    ↓
CoreManager
    ↓
Active Core
```

实时数据：

```text
Rust Event
    ↓
Tauri Event
    ↓
React Store
    ↓
UI
```

用于：

- Traffic
- Logs
- Core Status
- Connection Status

---

# 30. 开发阶段

## Phase 0：产品与架构设计

完成：

- 项目定位
- 技术选型
- Core 架构
- Unified Profile
- UI Information Architecture
- Design System

## Phase 1：Rust Core Manager MVP

实现：

- Mihomo Binary Detection
- sing-box Binary Detection
- Xray Binary Detection
- Start
- Stop
- Restart
- Status
- Log Capture

暂不实现：

- GUI
- TUN
- DNS
- Subscription

验收：

> 三个 Core 都可以被程序稳定启动和停止。

## Phase 2：Unified Profile

实现：

- Node Model
- Profile Model
- Parser
- Validator

支持：

- VLESS
- VMess
- Trojan
- Shadowsocks

## Phase 3：Config Compiler

实现：

- MihomoCompiler
- SingBoxCompiler
- XrayCompiler

验收：

> 同一个 Unified Profile 可以生成三个有效 Core Config。

## Phase 4：Smart Core Selector

实现：

- Capability Matrix
- Feature Detection
- Compatibility Check
- Score Engine
- Manual Override

## Phase 5：Tauri GUI

优先：

- Sidebar
- Dashboard
- Profile Management
- Proxy List
- Settings

## Phase 6：System Proxy

实现：

- Enable
- Disable
- Restore Previous Settings

验收：

> 程序退出后不会遗留错误的系统代理配置。

## Phase 7：TUN

实现：

- Permission Check
- Enable
- Disable
- Route Cleanup
- Crash Recovery

## Phase 8：DNS

实现：

- DNS Configuration
- Leak Protection
- Restore System DNS

## Phase 9：Subscription

实现：

- URL Import
- Auto Update
- Manual Update
- Cache
- Error Recovery

## Phase 10：高级功能

实现：

- Rules
- Traffic Charts
- Connection Manager
- Node Favorites
- Auto Test
- Core Update
- Application Update

---

# 31. MVP 功能范围

第一版正式可用版本：

- Mihomo
- sing-box
- Xray
- Smart Core Mode
- Manual Core Mode
- Profile Import
- Proxy Selection
- Latency Test
- System Proxy
- Modern Dashboard
- Logs
- Basic Subscription

暂时不做：

- 复杂规则编辑器
- 大量高级 DNS 设置
- 过多实验功能
- 过度自定义

---

# 32. 跨平台计划

建议优先：

```text
Windows
    ↓
macOS
    ↓
Linux
```

系统层通过 Platform Adapter 抽象。

需要重点处理：

- System Proxy
- Administrator Permission
- TUN
- Route
- DNS

---

# 33. 错误恢复策略

必须处理异常退出。

启动时执行：

```text
Application Start
        ↓
Check Previous State
        ↓
Was Core Running?
        ↓
Cleanup Old Network State
        ↓
Restore Safe State
```

记录：

- Active Core
- TUN Status
- System Proxy Status
- DNS Status

---

# 34. 性能目标

建议目标：

```text
Application Startup < 2 seconds

Core Switch < 3 seconds

UI Target: 60 FPS

Node List: 支持数百到数千节点
```

日志使用 Ring Buffer，避免无限增长。

---

# 35. 安全原则

禁止：

- 明文保存敏感凭据
- 自动信任未知配置
- 执行订阅中的任意脚本

必须：

- Config Validation
- Path Validation
- Process Argument Safety
- Secret Storage
- Permission Check

订阅处理：

```text
Download
    ↓
Parse
    ↓
Validate
    ↓
Normalize
    ↓
Store
```

---

# 36. 最终产品结构

```text
                    TriProxy
                       │
        ┌──────────────┼──────────────┐
        │              │              │
        ▼              ▼              ▼
   Modern UI      Smart Engine     Network
        │              │              │
        │              │         ┌────┼────┐
        │              │         ▼    ▼    ▼
        │              │       Proxy TUN DNS
        │              │
        │       ┌──────┼──────┐
        │       ▼      ▼      ▼
        │    Mihomo Sing-box Xray
        │
        ▼
Dashboard
Proxies
Profiles
Rules
Connections
Logs
Settings
```

---

# 37. 最终产品定位

本项目不是：

> 又一个配置文件启动器。

而应该是：

> **一个现代化的智能代理管理平台。**

用户体验：

```text
Import
   ↓
Analyze
   ↓
Optimize
   ↓
Connect
```

用户不需要理解：

- Core
- Config Format
- Protocol Compatibility
- DNS Implementation
- Routing Details

系统负责：

- Compatibility
- Core Selection
- Config Compilation
- Network Lifecycle

---

# 38. 当前最优开发顺序

```text
① Rust 项目基础结构
↓
② Core Trait
↓
③ Mihomo Adapter
↓
④ sing-box Adapter
↓
⑤ Xray Adapter
↓
⑥ CoreManager
↓
⑦ Unified Profile
↓
⑧ Config Compiler
↓
⑨ Smart Core Selector
↓
⑩ Tauri Backend Commands
↓
⑪ Modern UI
↓
⑫ System Proxy
↓
⑬ Subscription
↓
⑭ TUN
↓
⑮ DNS / Rules / Advanced Features
```

---

# 39. 下一步行动

现在进入：

```text
Phase 1
```

第一阶段目标：

> **创建一个真正可以运行的 Rust 多核心管理器。**

第一版结构：

```text
tri-proxy
├── MihomoCore
├── SingBoxCore
├── XrayCore
├── ProxyCore Trait
└── CoreManager
```

验收目标：

```text
tri-proxy start mihomo
tri-proxy stop
tri-proxy start sing-box
tri-proxy switch xray
tri-proxy status
```

三个核心能够：

- 启动
- 停止
- 重启
- 切换
- 读取日志
- 检查状态

当这个基础稳定后，再开始构建现代化 Tauri GUI。
