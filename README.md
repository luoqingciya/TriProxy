# TriProxy

一个基于 Rust 的现代化代理客户端，统一管理 **Mihomo / sing-box / Xray** 三个核心。
用户只需要「导入配置 → 选择节点 → 连接」，配置解析、协议识别、核心选择、配置转换、网络管理全部由程序完成。

> 完整产品规划见 [`docs/PROJECT_PLAN.md`](docs/PROJECT_PLAN.md)。
> 当前实现范围见 [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)。

---

## 当前进度

| 阶段 | 内容 | 状态 |
|---|---|---|
| Phase 0 | 产品与架构设计 | 已完成（规划书） |
| **Phase 1** | **Rust 多核心管理器 MVP + CLI** | **已完成并通过三核心真机验证** |
| Phase 2 | Unified Profile / Parser | 未开始 |
| Phase 3 | Config Compiler | 未开始 |
| Phase 4 | Smart Core Selector | 未开始 |
| Phase 5 | Tauri GUI | 未开始 |

Phase 1 的验收目标（规划书第 39 章）已全部达成：

```text
triproxy start mihomo      ✓
triproxy stop              ✓
triproxy start sing-box    ✓
triproxy switch xray       ✓
triproxy status            ✓
```

三个核心均可稳定 启动 / 停止 / 重启 / 切换 / 读取日志 / 检查状态。

---

## 快速开始

需要 Rust 1.82+ 与 Node 仅在前端阶段才需要。

```bash
git clone https://github.com/luoqingciya/TriProxy.git
cd TriProxy
cargo build --release
```

首次使用需要下载核心二进制（不随包分发，见 [`NOTICE.md`](NOTICE.md)）：

```bash
triproxy core install mihomo
triproxy core install sing-box
triproxy core install xray
```

国内网络直连 GitHub Release 通常不可用，可指定镜像或使用本地已下载的压缩包：

```bash
triproxy core install mihomo --mirror https://ghfast.top/
triproxy core install xray --file ./Xray-windows-64.zip
```

然后：

```bash
triproxy status            # 三个核心的状态一览
triproxy start sing-box    # 启动指定核心
triproxy switch xray       # 切换到另一个核心（自动停旧起新）
triproxy logs -f           # 跟踪日志
```

---

## 命令

| 命令 | 说明 |
|---|---|
| `triproxy status [--json]` | 管理器状态：活动核心、各核心状态/端口/延迟/二进制 |
| `triproxy cores [--json]` | 核心列表与二进制安装状态 |
| `triproxy start [core]` | 启动核心（省略则用上次的活动核心） |
| `triproxy stop` | 停止当前核心 |
| `triproxy restart` | 重启当前核心 |
| `triproxy switch <core>` | 停旧起新，输出可解释的切换步骤 |
| `triproxy logs [-n N] [--core X] [-f]` | 查看/跟踪核心日志 |
| `triproxy core install <core> [--version V] [--force] [--mirror P] [--file PATH]` | 下载、校验、安装核心 |
| `triproxy core path <core>` | 打印实际使用的二进制路径 |
| `triproxy doctor` | 环境自检：二进制可执行性、端口占用、数据目录可写 |

全局参数：`--data-dir <DIR>`（等价于环境变量 `TRIPROXY_HOME`）、`--json`。

数据目录默认为 `%APPDATA%\TriProxy`（Windows）/ `~/Library/Application Support/TriProxy`（macOS）/ `~/.local/share/TriProxy`（Linux）。

---

## 设计要点

**独占性**：任何时刻最多一个核心处于 `Running`。`start` 遇到其他核心在运行时直接拒绝并提示改用 `switch`，避免多个核心同时抢端口、抢 TUN、抢系统代理。

**状态机**：`Stopped → Starting → Running → Stopping → Stopped`，异常进入 `Failed`。只有终态允许启动。

**跨进程接管**：CLI 每次调用是独立进程，而核心进程必须常驻。管理器把 `pid` 持久化到 `state.json`，下次调用时接管仍在运行的进程，并改用 PID 探活；PID 已消失则清理陈旧状态。这是 CLI 形态必需的，GUI 单进程形态下自然退化为普通句柄管理。

**校验和不妥协**：二进制从官方 Release 下载，SHA256 取自 `api.github.com` 的 asset digest（回退 `.dgst` / `checksums.txt`），取不到校验和则拒绝安装（除非显式 `--allow-unverified`）。因此镜像只能造成失败，无法投毒。

**日志跨调用可见**：内存环形缓冲随进程消失，所以核心输出同时落到 `logs/<core>.log`，`triproxy logs` 读文件、`-f` 轮询追加。

---

## 目录结构

```text
TriProxy/
├── crates/
│   ├── triproxy-core/          # UI 无关的核心层（未来被 Tauri 后端复用）
│   │   └── src/
│   │       ├── core/           # CoreKind / ProxyCore trait / CoreManager
│   │       ├── adapters/       # mihomo / singbox / xray 三个方言
│   │       ├── binaries/       # 定位、下载、校验、解包
│   │       ├── process.rs      # 进程监督器（启动/就绪探测/停止/日志捕获）
│   │       ├── bootstrap.rs    # Phase 1 最小配置（非 Config Compiler）
│   │       ├── capability.rs   # 能力矩阵（Phase 4 的输入）
│   │       ├── logging.rs      # 环形缓冲 + 广播
│   │       ├── paths.rs        # 数据目录布局
│   │       └── state.rs        # 核心状态机
│   └── triproxy-cli/           # triproxy 命令行
└── docs/
    ├── PROJECT_PLAN.md
    └── ARCHITECTURE.md
```

`triproxy-core` 不依赖任何 UI 技术，Tauri 后端（Phase 5）只需把它作为依赖引入即可，无需重写业务逻辑。

---

## 测试

```bash
cargo test              # 46 个单元测试
cargo clippy --all-targets
```

---

## 授权

代码授权待定（建议 GPL-3.0-or-later，与上游核心生态一致）。
三个核心二进制不随本项目分发，其授权与来源见 [`NOTICE.md`](NOTICE.md)。

Copyright © luoqingci
