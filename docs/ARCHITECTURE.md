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

### 2.4 二进制不随包分发，且校验失败即拒绝

规划书 6 章把二进制放在 `src-tauri/binaries/`。实际改为**首次运行下载 + 强制校验**：

- 安装包体积从 100–200 MB 降到可接受范围；
- 不承担再分发 GPL/MPL 二进制的合规义务；
- 校验和取自 `api.github.com` 的 asset `digest`（回退 `.dgst` / `checksums.txt`），
  取不到就拒绝安装（`--allow-unverified` 是显式逃生舱，不是默认值）。

### 2.5 下载必须支持镜像与卡死检测

实测（2026-11，中国大陆网络）：直连 GitHub Release 资源约 90 B/s，22 MB 的 Mihomo 包
180 秒只下到 16 KB。因此：

- 引入 `--mirror` / `TRIPROXY_GITHUB_MIRROR`，候选顺序为「用户镜像 → 直连 → 内置镜像」；
- 下载改为流式 + 15 秒无进展即放弃并换下一个源（否则单次尝试就能挂几分钟，回退形同虚设）；
- 额外提供 `--file`，允许用户用任意方式下载后交给 TriProxy 安装。

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

## 5. 尚未实现（明确的边界）

Phase 1 刻意不做，避免半成品：

- 系统代理、TUN、DNS、路由（Phase 6–8）
- Unified Profile / Parser / Config Compiler（Phase 2–3）
- 订阅系统（Phase 9）
- 节点模型与延迟测试（Phase 2/4 依赖 Unified Profile）
- 控制器 API 健康检查（当前用 TCP 探活，Phase 4 再接 Mihomo/Clash API）
- GUI（Phase 5）

因此 `triproxy` 目前**不是一个可用的翻墙客户端**，而是一个可运行的、经真机验证的
多核心生命周期管理器。这是规划书 38 章规定的开发顺序。
