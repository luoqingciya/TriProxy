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

# ARCHITECTURE — Phase 1 实现说明

本文记录 Phase 1（Rust 多核心管理器 MVP）的实际架构，以及与 `PROJECT_PLAN.md` 的差异和原因。
规划书描述的是目标态，本文描述的是**已经落地并被验证的部分**。

---

## 1. 分层

```text
triproxy-cli                 命令行（未来替换为 Tauri 后端 + React UI）
      │
triproxy-core
      │
CoreManager  ── 独占性、切换流程、状态持久化、崩溃恢复
      │
ProxyCore    ── 统一 trait：version / check / start / stop / health / adopt
      │
CoreDialect  ── 每个核心的差异：参数、版本解析、配置渲染、能力声明
      │
Supervisor   ── 进程生命周期、就绪探测、日志捕获、PID 探活
      │
Mihomo / sing-box / Xray
```

关键约束：`triproxy-core` 不依赖任何 UI 技术。Phase 5 的 Tauri 后端只需把它作为依赖引入，
不需要重写任何业务逻辑。这正是规划书 3.3「UI 与 Core 完全解耦」的落地方式。

---

## 2. 与规划书的差异（重要）

### 2.1 `ProxyCore` 使用 `&self`，而非 `&mut self`

规划书 9 章的 trait 使用 `async fn start(&mut self, ...)`。落地时改为 `&self`，
状态放在实现内部的 `tokio::sync::Mutex` 里。

原因：`CoreManager` 需要持有 `Arc<dyn ProxyCore>`。若方法签名是 `&mut self`，Manager 只能写成
`Arc<Mutex<dyn ProxyCore>>`，形成「Manager 锁 → Core 锁」的嵌套，切换核心时极易死锁，
且无法并发查询三个核心的状态。改为 `&self` + 内部锁后，Manager 自己用一把 `op_lock`
串行化生命周期操作（对应规划书 14 章的「Lock CoreManager」），互斥关系只有一层。

### 2.2 新增 `adopt()`：跨进程接管

CLI 每次调用是独立进程，而核心进程必须常驻。若只在内存里持有 `Child` 句柄，
`triproxy start` 退出后 `status` / `stop` 就彻底失效。

因此管理器把 `{active_core, pid, listen_port}` 持久化到 `state.json`，
下次调用时接管仍在运行的进程（`Supervisor::adopt`），并把存活判定从
`Child::try_wait()` 切换为 PID 探活（Windows 用 `tasklist`，Unix 用 `kill -0` / `/proc`）。
PID 已消失则清理陈旧状态。

GUI 是单进程长驻形态，接管逻辑自然退化为普通句柄管理，不会产生副作用。

### 2.3 `bootstrap.rs` 不是 Config Compiler

Phase 1 需要真实可用的配置才能验收「三个核心能稳定启停」，
但又不能提前把 Phase 3 的 Unified Profile 拉进来。因此单独设 `bootstrap` 模块，
只生成「直连 + 本地入站」的最小配置，并在代码注释里明确标注：

> 这是 Phase 1 的占位实现，Phase 3 的 `ConfigCompiler` 会整体替换它，
> 期间不得让它长成第二个编译器。

生成的配置会先经过各核心自己的校验命令（`mihomo -t` / `sing-box check` / `xray -test`），
校验失败即视为启动失败，不做「先跑起来再说」。

### 2.4 核心二进制：默认不带，发布时可选带

规划书 6 章把二进制放在 `src-tauri/binaries/`。实际拆成两种发行形态：

- **`lite`（默认）**：不随包分发，首次运行下载 + 强制校验。安装包小，不承担再分发义务。
- **`full`**：由 GitHub Actions 在打包时下载并校验后放入 `cores/`，随包分发未修改的上游构建。

校验和始终取自 `api.github.com` 的 asset `digest`（回退 `.dgst` / `checksums.txt`），
取不到就拒绝安装（`--allow-unverified` 是显式逃生舱，不是默认值）。
发布流水线复用同一套下载与校验代码，因此 CI 打包与终端用户下载受同等保护。

配套的解析顺序（`BinarySource`）：

```text
env(TRIPROXY_*_BIN) → managed(<data>/bin) → bundled(<exe_dir>/cores) → PATH
```

`managed` 排在 `bundled` 之前：用户用 `core install` 更新过核心后，不应悄悄回落到
随包的旧版本。`cores/` 在运行时只读，所有写入都进 `bin/`。
版本展示不需要执行核心——`full` 包同时附带安装记录（`cores/<core>.json`），
`status` 直接读记录即可显示版本与来源。

### 2.5 下载必须支持镜像、卡死检测与吞吐下限

实测（2026-11，中国大陆网络）：直连 GitHub Release 资源约 90 B/s，22 MB 的 Mihomo 包
180 秒只下到 16 KB。因此：

- 引入 `--mirror` / `TRIPROXY_GITHUB_MIRROR`，候选顺序为「用户镜像 → 直连 → 内置镜像」；
- 下载改为流式 + 15 秒无进展即放弃并换下一个源；
- **额外加 40 kB/s 的平均吞吐下限**（20 秒宽限期后生效），300 秒为兜底上限；
- 额外提供 `--file`，允许用户用任意方式下载后交给 TriProxy 安装。

吞吐下限是发布流水线干跑时补上的：最初只有「无进展」判定，结果一个 90 B/s 的源
每 14 秒还能挤出几个字节，卡死计时器永远不触发，单次尝试被整体超时拖了 10 分钟，
回退链形同虚设。只判断「有没有数据」不够，必须判断「速率是否可用」。

由于校验和来自 `api.github.com` 而非镜像，镜像只能造成失败，无法投毒。

### 2.6 日志双写

内存环形缓冲随进程销毁，跨调用不可见。核心输出同时写入 `logs/<core>.log`
（`<epoch_ms>\t<level>\t<text>`），`triproxy logs` 读文件，`-f` 轮询追加。
超过 8 MB 轮转一代。

### 2.7 CLI 退出方式

启动核心后 CLI 必须立即返回，但 tokio 的进程驱动器在运行时销毁时会等待它派生过的子进程。
普通 `return` 会导致 `triproxy start` 永不返回（实测挂起 3 分钟以上）。
因此 `main` 在刷完 stdout/stderr 后直接 `std::process::exit`，跳过运行时析构。
这是「CLI 故意留下常驻守护进程」场景的标准做法。

### 2.8 Windows 句柄继承：一个会误导人的坑

`triproxy start | tee log` 这类管道用法最初会「挂死」：命令输出已经完整打印，核心也已启动，
但管道永不结束。实测确认——杀掉核心进程的瞬间管道立刻结束。

根因不在 CLI 退出，而在 Windows 的句柄继承：子进程创建时继承句柄是开启的，
TriProxy 自己从父进程（shell 管道 / IDE 终端 / 未来 GUI 的日志面板）继承来的 stdout
是可继承句柄，于是核心进程在整个生命周期里一直持有那个管道的写端，
读取方永远等不到 EOF。

修复：在 `spawn` 前对自身的 STD_INPUT/OUTPUT/ERROR 调用 `SetHandleInformation(HANDLE_FLAG_INHERIT, 0)`，
只清掉「继承来的」可继承标志；TriProxy 为核心创建的 stdout/stderr 管道不受影响，
它们本来就是为该子进程创建的可继承句柄。

这个坑对脚本化和 GUI 集成都会致命，因此记录在此，避免后续被当成「偶发卡顿」重新排查一遍。

---

## 3. 状态机与切换流程

```text
Stopped ──start──▶ Starting ──就绪──▶ Running ──stop──▶ Stopping ──▶ Stopped
                      │                                      │
                      └──────────── Failed ◀─────────────────┘
```

「就绪」的判定不是「进程还活着」，而是**本地入站端口能建立 TCP 连接**（默认 10 秒超时）。
启动阶段进程提前退出，会把输出尾部拼进错误信息一起报出。

`switch` 的步骤全部记录在 `SwitchReport.steps` 里并直接打印，对应规划书 14 章的
13 步流程与 23 章「自动决策必须可解释」。Phase 1 实际执行的子集：

```text
1. stop current core        （含等待进程退出）
2. cleanup network state    （Phase 1 无系统代理/TUN/DNS 状态需还原，显式声明）
3. generate config
4. validate config
5. start new core + health check
6. persist state
```

---

## 4. 能力矩阵（Phase 4 的输入）

`CoreCapability` 是静态声明而非运行时探测——这是「自动选择可解释」的前提。
`ProfileRequirement::unmet_by()` 实现了规划书 13 章的硬约束淘汰：
硬性不兼容直接出局，不参与加权评分。

当前矩阵（仅声明已确认的部分）：

| 能力 | Mihomo | sing-box | Xray |
|---|---|---|---|
| TUN | ✓ | ✓ | ✗（需外部工具） |
| DNS | ✓ | ✓ | ✓ |
| Reality / Vision | ✓ | ✓ | ✓ |
| 原生 rule-set | ✗（用 rule-provider） | ✓ | ✗ |
| proxy-groups | ✓ | ✓ | ✗ |

Xray 缺少原生 TUN 这一条会直接影响 Smart 模式的选型结果，是真实差异而非笔误。

---

## 5. 发布流水线

`.github/workflows/release.yml`，推送 `v*` tag 或手动触发。

矩阵只使用**原生 runner**：`triproxy core install` 按 `std::env::consts` 判断目标平台，
交叉编译会把错误架构的核心打进包，所以不做交叉构建。

```text
windows-latest  → x86_64-pc-windows-msvc
ubuntu-latest   → x86_64-unknown-linux-gnu
macos-latest    → aarch64-apple-darwin
macos-13        → x86_64-apple-darwin
```

每个任务产出两个包：

1. 编译 → `dist/lite/`（二进制 + README/LICENSE/NOTICE/ADDITIONAL-TERMS）
2. 用刚编译出的二进制执行 `core install mihomo|sing-box|xray --data-dir <tmp>`，
   把 `bin/` 下的二进制与安装记录复制进 `dist/full/cores/`
3. 用 `jq` 读安装记录生成 `cores/THIRD-PARTY-NOTICES.txt`（版本 / 授权 / 源码 tag 地址 / sha256）
4. **冒烟验证**：用全新数据目录运行 `dist/full/triproxy cores`，断言输出含 `bundled`。
   这一步能拦住「包做出来了但程序找不到核心」这类只在发布后才暴露的问题
5. 打包（Windows `zip` / Unix `tar.gz`）并上传 artifact

release 任务汇总所有 artifact，`gh release create` 发布，说明取自
`docs/RELEASE_NOTES_TEMPLATE.md`（两种包的取舍、lite 版安装指引、授权与随包核心的合规说明）。

## 6. 尚未实现（明确的边界）

Phase 1 刻意不做，避免半成品：

- 系统代理、TUN、DNS、路由（Phase 6–8）
- Unified Profile / Parser / Config Compiler（Phase 2–3）
- 订阅系统（Phase 9）
- 节点模型与延迟测试（Phase 2/4 依赖 Unified Profile）
- 控制器 API 健康检查（当前用 TCP 探活，Phase 4 再接 Mihomo/Clash API）
- GUI（Phase 5）

因此 `triproxy` 目前**不是一个可用的翻墙客户端**，而是一个可运行的、经真机验证的
多核心生命周期管理器。这是规划书 38 章规定的开发顺序。
