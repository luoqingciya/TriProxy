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
```

macOS Intel（`x86_64-apple-darwin`）**不在支持范围**：Apple 已停止销售 Intel Mac，
GitHub 的 Intel runner 也长期排队，维护一条几乎无人使用的构建线不划算。
`binaries/manifest.rs` 仍保留该目标的资产匹配规则，用户自行编译时可以正常使用，
只是不发布官方包。

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

## 6. Phase 2 — Unified Profile

```text
订阅文本 / 分享链接
      ↓  profile::parse_payload
Profile { id, name, metadata, nodes, groups }
      ↓  （Phase 3）
mihomo.yaml / sing-box.json / xray.json
```

### 6.1 模型

`Node` 由「协议 + 地址 + 端口 + 凭据 + 传输 + TLS」组成，`Credentials` 用枚举区分四种协议，
`Transport` 覆盖 tcp / ws / grpc / h2，`TlsConfig` 里 `reality: Option<RealityConfig>`
表示 Reality。VLESS 的 `flow` 单独建模为 `VlessFlow::{None, Vision}`，而不是塞进 `Transport`，
因为它是凭据层语义（对应能力矩阵里的 `supports_vision`）。

**`rules` / `dns` / `network` 没有建模**，尽管规划书的 `Profile` 结构里列了它们。
理由：这三者只有在消费它们的编译器存在（Phase 3）之后才有确定形状——
Clash 的 rule-provider 与 sing-box 的 rule-set 不是同一个概念，提前拍一个统一模型必然返工。
`groups` 已定义（形状简单且确定），Phase 3 填充。

### 6.2 节点身份与显示名解耦

`Node::id` 是连接参数的哈希（协议、地址小写化、端口、凭据、传输），**不含显示名**。
服务商给节点改名不会产生新节点，收藏和延迟历史因此不会丢。
`Profile::id` 是排序后节点 id 的哈希，同一份订阅重复导入得到同一个 id（Phase 9 去重的基础）。

名称唯一性在解析阶段强制：Clash 与 sing-box 都以名称作为代理的键，重名会互相覆盖，
所以重名节点自动加 ` (2)` 后缀，而不是让导入失败。

### 6.3 分享链接的历史包袱

四种协议没有统一的 URI 规范，解析器是手写的（没有用通用 URL 库，因为 `ss://` 的
legacy 形态根本不是合法 URL）：

- **`ss://` 两种格式**：SIP002（`base64(method:password)@host:port`）与 legacy
  （整个 body 是 `base64(method:password@host:port)`）。用「有没有 `@`」区分；
  userinfo 里有没有 `:` 再决定要不要 base64 解码——base64 字母表不含 `:`，这个判定无歧义。
- **`vmess://` 是 base64 的 JSON**，且 `port` / `aid` 在不同客户端里可能是字符串或数字，
  所以用 `serde_json::Value` 再强制转换，而不是直接声明成 `u16`。
- **query 键名不统一**：`serviceName` / `path`、`security` / `tls`、`allowInsecure` 大小写不一。
  键统一转小写后按别名匹配。
- **不支持的传输/加密/flow 直接报错**，不静默降级。例如 `net=kcp` 的 VMess 节点会被拒绝并
  说明原因，而不是编译成一个跑不通的配置。

### 6.4 诊断而非失败

`parse_payload` 永不 panic，也永不因为单行失败而丢弃整份订阅：坏行变成带行号的 `Diagnostic`，
其余节点照常导入。校验同样返回诊断列表，分 `Error`（节点不可用）与 `Warning`
（能连但大概率不是用户想要的，如 `allowInsecure`、Reality 公钥长度异常、Vision 无 TLS）。

`profile check` 在有 `Error` 时退出码为 1，便于脚本化。

### 6.5 与 Phase 4 的接口

`Profile::requirement()` 直接产出 `ProfileRequirement`，供 Smart Core Selector 使用。
只报告节点层面可见的需求（协议、Reality、Vision、WireGuard）；TUN / DNS / rule-set
属于网络与规则配置，Phase 3 才有。

## 7. Phase 3 — Config Compiler

```text
Profile
  ├─→ MihomoCompiler   → config.yaml   (YAML)
  ├─→ SingBoxCompiler  → config.json   (JSON)
  └─→ XrayCompiler     → config.json   (JSON)
```

### 7.1 三条设计规则

1. **无法忠实表达就跳过，不做近似**。静默丢掉一个传输选项会生成「能连但行为不同」的配置，
   比一个明显缺失的节点危险得多。被跳过的节点带原因记录在 `CompiledConfig::skipped` 里。
2. **硬能力不匹配直接整体失败**。`compile()` 先跑 `ProfileRequirement::unmet_by()`，
   核心根本服务不了这份 Profile 时不出配置（规划书 13 章）。
3. **生成的配置必须先由核心自己校验**。`CoreManager` 写盘后立刻调用
   `mihomo -t` / `sing-box check` / `xray -test`，编译器有 bug 会表现为「配置被拒绝」，
   而不是「核心起来了但行为不对」。

### 7.2 真实核心教会我们的事

Xray 编译器最初把 Reality + WebSocket 节点照直写进 `streamSettings`，被 Xray 自己否掉了：

```text
REALITY only supports RAW, XHTTP and gRPC for now.
```

这是编译期就该知道的约束，已改为在编译时跳过并说明原因。mihomo 与 sing-box 都接受该组合，
所以只在 Xray 编译器中限制——**每个核心的限制来自该核心自己的校验器，不靠猜**。

同类约束还有：trojan 没有 TLS 就不是 trojan（三个编译器统一拒绝），
Xray 不支持 shadowsocks plugin（跳过），sing-box 反而原生支持 SIP003 plugin（透传）。

### 7.3 各核心的结构差异

| | Mihomo | sing-box | Xray |
|---|---|---|---|
| 节点选择 | `proxy-groups: select` | `selector` outbound | **无**，只有第一个 outbound 生效 |
| 传输 | `network` + `ws-opts`/`grpc-opts`/`h2-opts` | `transport` 对象 | `streamSettings` |
| Reality | `reality-opts` | `tls.reality` | `streamSettings.realitySettings` |
| 本地入站 | `mixed-port` | `mixed` inbound | socks + http 两个入站（相邻端口） |

Xray 没有 selector，所以多节点时只有第一个被使用。这是核心能力差异，不是实现缺陷；
节点选择属于 Phase 4，而 Mihomo/sing-box 已经提供了可用的选择器。

### 7.4 YAML 是手写的

Mihomo 需要 YAML 输出，但项目只**写**不读，而维护中的序列化库都会带上完整解析器。
因此写了 `compiler::yaml`：只支持 map / list / scalar，所有字符串一律双引号转义。
节点名来自订阅，可能含 `:`、`#`、引号、CJK 与 emoji——裸写会静默改变配置语义。

### 7.5 与 CLI / 管理器的衔接

```bash
triproxy profile compile sub.txt --core xray -o xray.json --check
triproxy start mihomo --profile sub.txt
triproxy switch xray --profile sub.txt
```

`CoreManager::start_with_config` / `switch_with_config` 接收已渲染的配置文本；
`validate_config` 写到 `downloads/` 而不是活动配置目录——运行中的核心会监视自己的配置文件并热重载，
校验用的临时文件不能放在那里。

`bootstrap.rs` 仍然保留：不带 `--profile` 的 `start` 还没有 Profile 可用（订阅存储属于 Phase 9），
所以「直连最小配置」仍是默认路径。Phase 9 落地后这条路径应被删除。

## 8. Phase 4 — Smart Core Selector

### 8.1 没有实现规划书的加权公式，原因如下

规划书 13 章给出 `Compatibility 50% / Feature Match 25% / Performance 15% / User Preference 10%`。
这个公式没有落地，因为它有两个站不住的地方：

- **兼容性不能是权重**。跑不了这份 Profile 的核心不是「90% 兼容」，而是不可用。
  硬约束淘汰在前，留下的候选兼容性都是 100%，那 50% 的权重贡献恒为常数。
- **性能没有测量依据**。给一个没人测过的数字配 15% 权重，只会让结果看起来精确，实则任意。

### 8.2 实际采用的字典序排序

1. **硬兼容性淘汰**（`ProfileRequirement::unmet_by`）
2. **原生特性数量**：Profile 要求的可选特性里，该核心有几个是一等公民
3. **默认顺序**：sing-box → mihomo → xray

第 3 条不是性能排名，而是照搬规划书 10 章自己的定位：sing-box 是「通用现代核心」，
mihomo 面向 Clash 生态，Xray 面向 V2Ray/Xray 生态。没有生态偏好的 Profile，
默认给通用核心是更稳妥的选择。这三条全部写死在代码与文档里，同一个 Profile 永远选出同一个核心。

### 8.3 只有「差异化」特性才计分

Reality 与 Vision 三个核心都支持，把它们算作「原生特性」会给每个候选加同一个常数，
不携带任何信息。只有 `rule-set`（仅 sing-box 原生）与 `proxy-groups`（sing-box、mihomo）
参与比较。

### 8.4 应用侧需求必须单独建模

`Profile::requirement()` 只能从节点推导出协议、Reality、Vision、WireGuard。
TUN、DNS、rule-set 属于应用设置，节点里没有这些信息。如果只吃 Profile，
`requires_tun` 恒为 false，Xray 永远看起来同样合适——选型就退化成纯 tie-break。

因此引入 `NetworkRequirements { tun, dns, rule_set, proxy_groups }`，
由调用方（未来的 GUI 设置页）提供，`Selector::requirement()` 把它并进 Profile 需求。
`core select --tun` 就是这条路径的演示：打开 TUN 后 Xray 立刻被判为 ineligible。

### 8.5 解释必须包含被淘汰者

```text
  CORE      VERDICT     DETAIL
  sing-box  selected    -
  mihomo    eligible    -
  xray      ineligible  missing TUN

Reason: sing-box is the default general-purpose core; no candidate needed a
specialisation; eliminated: xray (TUN)
```

只说「为什么是它」不够，还要说「为什么不是另外两个」。核心从列表里安静消失，
正是最需要解释的那类决策（规划书 23 章）。

### 8.6 手动覆盖

`CorePreference::Fixed(kind)` 仍然会跑兼容性检查：用户手选一个跑不了这份 Profile 的核心时
直接报错并说明缺什么，而不是悄悄回退到别的核心——回退会让用户以为自己选的生效了。

## 9. 存储层（Profile 库）

GUI 需要 Profile 列表，而在此之前 Profile 只活在一次命令的生命周期里。这一层是从规划书
Phase 9（订阅系统）提前拉过来的，只做「存/取/选中」，不做下载与自动更新。

### 9.1 为什么不是 SQLite

规划书 18 章建议 SQLite。这里先用「一个 Profile 一个 JSON 文件 + 一个索引文件」：

- Profile 是整体读、整体写的对象，没有需要 join 或按条件筛选的字段；
- 数量在几十份量级，遍历目录完全够用；
- 出问题时可以直接打开文件看，备份就是复制目录。

SQLite 会在有**真正需要查询的数据**时才有价值——连接历史、延迟采样、节点使用频次。
那是后续阶段的事，现在引入数据库只会增加迁移负担。这个判断写在 `store.rs` 顶部注释里，
避免以后有人以为是漏了。

### 9.2 两个 id，用途不同

| | 来源 | 稳定性 | 用途 |
|---|---|---|---|
| `Profile::id` | 节点连接参数的哈希 | 节点变化就变 | 内容指纹，用于去重 |
| `StoredProfile::id` | 首次导入时的名称+时间 | 内容更新后不变 | 记录身份，用于「活动 Profile」与 UI 位置 |

如果只有一个 id，订阅刷新（节点变了）会产生一条新记录，用户的活动 Profile 就丢了。

`profile add` 遇到同名记录时按名称匹配并**更新**该记录，而不是新建一条同名副本。

### 9.3 损坏文件不拖垮整个库

`list()` 跳过无法解析的文件并记一条 warn，而不是返回错误。
一个坏文件让用户看不到其余 30 份订阅是不可接受的失败模式。
索引里的悬空 `active` 指向也会被自动清理。

## 10. Phase 5 — Tauri GUI

```text
React + TypeScript + Tailwind   (src/)
        ↓  invoke()
Tauri commands                  (src-tauri/src/commands.rs)
        ↓
triproxy-core                   （与 CLI 完全相同的代码）
```

### 10.1 后端命令层刻意很薄

`src-tauri/src/commands.rs` 里每个命令只对应一个 core 操作，没有业务规则。
选核、编译、独占性、崩溃恢复全在 `triproxy-core` 里——否则 CLI 与 GUI 会各自演化出
不同的行为，而用户会认为它们是同一个产品。

### 10.2 Tauri 应用是独立的 Cargo workspace

根工作区 `exclude = ["src-tauri"]`。原因：GUI 依赖平台 webview 工具链
（WebView2 / WebKitGTK），如果把它并入根工作区，`cargo test --all` 和 CI 都会被迫安装这些依赖，
而单元测试根本不需要它们。

### 10.3 前端可以在浏览器里开发

`npm run dev` 打开的是一个普通网页，没有 Rust 后端。因此 `services/api.ts` 判断
`__TAURI_INTERNALS__` 是否存在：不在 Tauri 里就走 `services/mock.ts` 的假数据。
这让界面可以在不构建桌面程序的情况下迭代与评审，桌面版永远走真实命令。

### 10.4 界面验证方式

本轮不是「构建成功就算完成」：用 Edge headless 对每个页面截图后逐一检查排版
（列对齐、文字裁切、明暗主题对比度），据此修了三处真实缺陷——仪表盘底部大片空白、
日志页高度没撑满、核心表格列宽把「监听」推得过远。

### 10.5 与规划书的差异

- **没有用 React Router**。桌面窗口没有地址栏，5 个页面不需要路由库；
  页面状态放在 store 里并同步到 URL hash，这样刷新后仍停在原页面。
- **没有虚拟列表**。当前节点数量在几十到几百，普通列表足够；
  等真的出现数千节点再上虚拟化（规划书 24 章的建议针对的是那种规模）。
- **规则 / 连接两个页面未实现**。它们需要 Phase 10 的后端能力，
  放一个空页面比不放更糟。

### 10.6 设置放在 core 而不是 GUI

`AppSettings`（主题、核心偏好、TUN/rule-set/proxy-groups、allow_lan、日志级别）位于
`triproxy-core`，读写 `<data>/settings.json`。这样 GUI 里打开的 TUN 会同样影响
`triproxy start`——两个前端共用一份设置，不会各说各话。

其中 `NetworkRequirements` 就是 Phase 4 特意留出的接口：设置页的 TUN / rule-set 开关
直接决定选核时淘汰哪些核心。

## 11. Phase 6 — 系统代理

### 11.1 难点不是设置，而是不留下烂摊子

改系统代理是一行注册表写入。真正的问题是**崩溃**：进程在「已改系统代理」和「恢复」之间
被杀掉，用户的整台机器就指向了一个已经不存在的端口——浏览器、更新程序、其他应用全部断网，
而且用户根本不知道是 TriProxy 干的。

三条规则保证这一点：

1. **先落盘，再改系统**。快照写入 `<data>/system-proxy.json` 之后才动注册表。
   若在两者之间崩溃，快照里的「原始值」与当前系统状态一致，恢复它是无害的（幂等）。
2. **重复启用不会污染快照**。第二次 `enable` 只更新 endpoint，`previous` 保持第一次捕获的值。
   否则快照会记成我们自己写的 `127.0.0.1:7890`，之后「恢复」就等于没恢复。
3. **启动即自愈**。CLI 与 GUI 启动时都会检查：存在快照但没有任何核心在运行 →
   这是上次崩溃的残留，立即恢复。

第 3 条的判定条件是「快照存在 **且** 核心没在跑」。只看快照会误伤：CLI 每次调用都是独立进程，
`triproxy proxy on` 之后紧接着的 `triproxy status` 如果无条件恢复，代理就白开了。

### 11.2 拒绝指向死端口

`enable()` 在改注册表之前先做一次 TCP 探活（500ms）。把一个已经不在监听的端口写进系统代理，
比不改更糟——用户会以为代理生效了，实际是全部断网。

### 11.3 平台实现

| 平台 | 方式 | 验证状态 |
|---|---|---|
| Windows | `HKCU\...\Internet Settings` + `InternetSetOption` 刷新 WinINET | **已在本机实测** |
| macOS | `networksetup` 逐服务设置 | 未验证 |
| Linux | `gsettings org.gnome.system.proxy` | 未验证 |

Windows 必须调用 `InternetSetOption(INTERNET_OPTION_SETTINGS_CHANGED / REFRESH)`，
否则已经在运行的应用会继续用旧代理，用户看到的现象是「开关没反应」。

macOS 与 Linux 的实现按同一 trait 编写，但**没有在真实机器上验证过**——这点在 PR 里明确标注，
不要当成已完成的能力。

### 11.4 实测结果（Windows）

```text
启用前：ProxyServer = 127.0.0.1:10808   （用户原本的代理）
proxy on  → 127.0.0.1:7890，快照记录 10808
proxy off → ProxyServer 回到 127.0.0.1:10808，ProxyOverride 逐字一致，快照删除

崩溃恢复：
proxy on → 外部杀掉核心进程（模拟崩溃，注册表仍指向 7890）
下一次任意命令 → "restored the system proxy left behind by a previous session"
                → 注册表回到 10808
```

## 12. Phase 7 — TUN 模式

### 12.1 TriProxy 不自己创建 TUN 设备

TUN 网卡由核心自己创建（Windows 用 wintun、macOS 用 utun、Linux 用 `/dev/net/tun`），
TriProxy 只负责**生成配置**、**检查前提**、**发现残留**。自己实现一套 TUN 只会多一层
与核心抢设备的风险，没有任何收益。

### 12.2 三件事归本模块所有

1. **配置生成**：sing-box 的 `tun` inbound（`interface_name: TriProxy`）、mihomo 的 `tun` 段。
   Xray 没有 TUN，`compile()` 在 `options.tun` 为真时直接返回 `Incompatible`——
   而不是生成一份核心会拒绝的配置。
2. **权限前置检查**：创建 TUN 需要管理员/root。启动前检查，把「核心启动失败 + 一段看不懂的
   日志」变成一句可执行的提示。
3. **残留发现**：核心被硬杀后网卡与其路由可能残留。`leftovers()` 枚举疑似我们的网卡并给出
   清理命令。

### 12.3 为什么只报告、不自动删除

删网卡是影响全系统的管理员操作。按名字匹配（`TriProxy` / `Mihomo` / `sing-box` 前缀）会有
误判可能——用户可能同时装着别的客户端。**误删一个正在使用的网卡，比留下一张闲置网卡糟得多**，
所以这里只报告并给出确切命令，由用户决定。

命名检测依赖编译器把 `INTERFACE_NAME` 写进配置；这也是为什么 sing-box 用 `interface_name`、
mihomo 用 `device` 固定成同一个名字。

**踩过的坑**：最初只查注册表 `...Network{4d36e972-...}<guid>ConnectionName`，
结果每次 TUN 正常退出后都会报「残留网卡 1 个」。原因是 Windows 移除网卡后**保留 Connection 键**
（用于设备回来时恢复每网卡设置）。注册表不是权威来源，现在每个命中项都会再用
`netsh interface show interface name=<name>` 的**退出码**确认网卡是否真实存在
（只看退出码不看输出，因此不受系统语言影响）。

### 12.4 验证边界（重要）

本机**没有管理员权限**，因此：

| 项目 | 状态 |
|---|---|
| sing-box 接受 TUN 配置（`sing-box check`） | ✅ 已验证 |
| mihomo 接受 TUN 配置（`mihomo -t`） | ✅ 已验证 |
| Xray + TUN 被能力矩阵拒绝 | ✅ 已验证 |
| 无管理员权限时给出明确错误 | ✅ 已验证 |
| 残留网卡检测 | ✅ 已验证（本机有 TAP 网卡，未被误判） |
| **真正创建 TUN 设备** | ✅ 管理员实测 |
| **崩溃后网卡与路由是否被清理** | ✅ 管理员实测 |

验收脚本：`scripts/verify-tun.ps1`（需以管理员身份运行）。它会拉起 TUN、记录网卡与路由、
硬杀核心模拟崩溃、再检查残留，最后清理。

实测结果（Windows 11，sing-box 1.14.4）：

```text
TUN 运行中：adapter TriProxy  status=Up  ifIndex=70  driver="sing-tun Tunnel"
           该网卡上 7 条路由，含 0.0.0.0/0 -> 172.19.0.2（默认路由被接管）

硬杀进程后：RESULT: adapter gone
           RESULT: ipv4 routes now 34 (baseline 34)
           RESULT: default routes via WLAN
```

即规划书 Phase 7 的验收项「程序退出后不会遗留错误的系统路由表」成立。

因此 Phase 7 标记为「部分完成」。把「配置能被核心接受」当成「TUN 能用」是这类功能最常见的
误判，这里不这么算。

## 13. Phase 8 — DNS

### 13.1 三种模式

| 模式 | 行为 |
|---|---|
| `off` | 不生成 `dns` 段，核心用自己的默认值 |
| `split` | 被代理的域名走隧道内解析，其余走本地解析器 |
| `fakeip` | 被代理的域名直接返回假 IP，真实域名在隧道内解析 |

**TUN 打开时 `off` 会被提升为 `split`**。原因不是洁癖：没有 DNS 段与劫持规则，
TUN 只是把发往原解析器的 UDP 包一起塞进隧道——慢，而且查询内容依然暴露。
这个提升写在 `DnsMode::effective()` 里，有测试锁定。

### 13.2 防泄漏靠的是「查询走隧道」，不是「换个解析器」

- **mihomo**：nameserver 带 `#PROXY` 后缀，查询经代理组发出
- **sing-box**：dns server 带 `detour: PROXY`，并用 `hijack-dns` 让核心自己应答

两者的共同点是**查询本身走了隧道**。只把系统 DNS 改成 1.1.1.1 之类的「换个解析器」，
查询依然从本地网络发出，只是换了个观察者——那不算防泄漏。

Xray 没有 `detour` 等价物，只能发出 dns 段，查询按路由规则走；它的防泄漏能力弱于另外两个核心。

### 13.3 为什么没有实现「恢复系统 DNS」

规划书 Phase 8 列了「Restore System DNS」。**没有实现，理由如下**：

- **有 TUN 时不需要**：`hijack-dns` 之后所有 53 端口流量都由核心应答，系统 DNS 指向哪里都无所谓。
- **没有 TUN 时没意义**：改系统 DNS 只是换解析器，查询仍然从本地网络发出，不解决泄漏。
- **代价很高**：改系统 DNS 需要管理员权限，且一旦核心异常退出，机器会进入「DNS 全挂」状态——
  比系统代理失效严重得多（浏览器至少还会报错，DNS 挂掉是整个网络看起来都坏了）。

为达成一个它其实达不成目标的特性，去引入一个能让整台机器失联的系统级副作用，不划算。
如果以后要做，必须复用系统代理那套「先落盘快照、崩溃自愈」的机制，并且只在用户明确开启时生效。

### 13.4 真实核心教会的事

sing-box 1.12+ 在生成 dns 段后要求 `route.default_domain_resolver`，否则直接拒绝启动：

```text
ERROR[0000] missing `route.default_domain_resolver`
```

因为 outbound 里的域名（比如节点地址）需要在任何规则生效之前先被解析。
已补上指向 `local-dns` 的引导解析器。

三核心 × 三模式共 9 种组合（含 `off`），全部通过各自核心的 `check` / `-t` 校验。

## 14. Phase 9 — 订阅

### 14.1 一条铁律：刷新失败不能毁掉已有的东西

服务商会挂、URL 会过期、网络会抖。因为一次请求超时就把用户能用的节点列表清空，
是这类功能最糟糕的失败方式。所以：

- **首次导入必须成功**——没有旧数据可退，存一份空配置比报错更糟；
- **刷新失败只记录错误**——`last_error` 写进订阅状态，profile 一个字节都不动；
- **失败也推进时间戳**——否则每次启动都会反复捶一个已经挂掉的服务器。

实测（本地 HTTP 服务器）：

```text
刷新失败 → `demo` — refresh failed: cannot reach http://127.0.0.1:18080/sub
          the stored profile is unchanged
          exit=1
之后 `profile show demo` → 3 nodes，完好
记录里 → last_error 已写入，last_fetch_ms 已推进
```

### 14.2 缓存靠条件请求，不是靠时间

订阅里的节点列表很少变，每次启动重新下载一兆再丢掉，既慢又对服务商不礼貌。
`ETag` 与 `Last-Modified` 随订阅一起存盘，下次请求带上 `If-None-Match` / `If-Modified-Since`；
服务端回 304 就只更新时间戳，不重新解析。

实测：第二次刷新返回 304 → `unchanged (provider returned 304)`；
服务端内容变更后 → `updated — 3 nodes`。

### 14.3 记录身份与订阅状态分开存

`StoredProfile::id` 在订阅刷新后不变（Phase 5 的设计），订阅状态作为 `Record` 的可选字段
一起落盘。老记录没有这个字段也能正常加载（`#[serde(default)]`），不需要迁移。

### 14.4 自动更新的触发点

`Subscription::is_due()` 判断是否到期，`interval_secs = 0` 表示不自动更新。
`refresh_due()` 只刷新到期的订阅；手动刷新（`profile refresh <name>` / GUI 的刷新按钮）
不看间隔。

**没有做后台常驻定时器**：CLI 每次调用都是独立进程，GUI 也没有常驻调度。
自动更新目前发生在显式调用 `refresh --all` 时。真正的后台更新需要一个常驻进程或系统服务，
那是另一个量级的工程，不在本阶段范围内——这一点写在这里，避免被当成已完成。

## 15. Phase 10 — 分流规则

### 15.1 两种模式

| 模式 | 行为 |
|---|---|
| `global` | 全部走代理（编译器的历史行为，默认） |
| `split` | 局域网与国内目标直连，其余走代理 |

默认仍是 `global`：静默改变既有行为比多一个开关更糟。

### 15.2 三个核心的实现方式完全不同

| 核心 | 做法 | 依赖 |
|---|---|---|
| mihomo | `GEOSITE,cn` / `GEOIP,CN` 规则 | 自带 geo 数据库 |
| sing-box | 远程 rule-set（`geosite-cn.srs` / `geoip-cn.srs`） | **运行时下载** |
| Xray | `geosite:cn` / `geoip:cn` 路由规则 | `geosite.dat` / `geoip.dat` |

sing-box 1.12 移除了内置 geoip/geosite，只能走 rule-set；其 `download_detour` 设为 `direct`——
用代理去下载「决定什么走代理」的规则集是个死循环。

### 15.3 安装器的一个真实缺口（这次才发现）

Xray 的 geo 分流依赖 `geosite.dat` 与 `geoip.dat`，它们就在官方发布包里，
但安装器只解出了可执行文件。结果是：**配置能被接受，运行时却报 `failed to open geosite.dat`**。

这不是配置问题，是安装器漏了文件。已修：`extract_bundle()` 现在同时解出已知的运行时数据文件
（`geosite.dat`、`geoip.dat`、`geoip-only-cn-private.dat`、`Country.mmdb`）写到可执行文件旁边。
tar.gz 分支也改成**扫描完整个归档再返回**，否则数据文件排在可执行文件之后时会被静默丢掉。

重装后实测：`geosite.dat` 7.3 MB、`geoip.dat` 3.2 MB 落盘，Xray 的 split 配置通过校验。

### 15.4 验证

三核心 × 两模式共 6 种组合，全部通过各自核心的 `check` / `-t`。

## 16. Phase 10 — 延迟测试

### 16.1 测的是什么，不是什么

探针是到节点 `server:port` 的 **TCP 握手**。它回答「这个端点能不能连上、有多远」——
这正是用户看延迟列想知道的事。

它**不能证明代理可用**：凭据错误、UUID 过期、加密方式不匹配，TCP 握手一样会成功。
要证明代理真的能用，需要通过运行中的核心发一次 HTTP 请求，那需要控制器 API，
而 Xray 根本没有——所以两者是**不同的测量**，不能用一个冒充另一个。

界面与 CLI 都把这句写出来了（「TCP handshake only, not proof the proxy works」），
不在数字旁边留模糊空间。

### 16.2 几个实现细节

- **并发 8**：一百个节点不至于要等一分钟，也不会让订阅服务商看到像攻击的突发。
- **IPv4 优先**：双栈节点很常见，IPv6 路由坏掉时不能把一个健康的 IPv4 端点报成不可达。
- **结果不落盘**：延迟是「此刻」的属性，存下来只会让界面拿旧数字冒充当前状态。

### 16.3 实测

```text
  #  NAME           ENDPOINT           TCP     STATUS
  1  AliDNS         223.5.5.5:443      -       connect timed out
  2  Baidu          www.baidu.com:443  70 ms   ok
  3  GitHub         github.com:443     129 ms  ok
  4  TEST-NET dead  192.0.2.1:443      -       connect timed out
```

`223.5.5.5:443` 超时是正确的——它是 DNS 服务器，只在 53 端口监听。
`192.0.2.1` 是 RFC 5737 保留地址，必然不可达。两个「失败」都是真的。

同样的原因，`1.1.1.1:443` 与 `8.8.8.8:443` 在本机也超时：这是网络现状，不是探针的 bug——
`127.0.0.1:1` 能立刻返回「连接被拒绝」，说明逻辑本身是通的。

## 17. Phase 10 — 核心更新

### 17.1 版本比较不能用字符串

三个项目的版本号形态不同（`1.19.33`、`1.14.4`、`26.10.31`，有的还带 `v` 前缀或 `-beta.1` 后缀）。
按字符串比较会把 `1.19.4` 判成比 `1.19.33` 新——这是最典型的版本号 bug。

`compare_versions()` 逐段按数字比较，并且**遇到非纯数字的段就停止**：
`1.15.0-beta.1` 的段是 `[1, 15, 0]` 加一个「预发布」标记，因此它比 `1.15.0` **低**。
这一点是写测试时才发现的：最初把 `-beta.1` 当成第 4 段解析，结果每个 beta 都被判成可更新。

### 17.2 网络失败不能报成「已是最新」

`check_update()` 把网络错误原样抛出，CLI 打印「cannot check for updates」。
如果吞掉错误返回「已是最新」，用户会基于一个假结论做决定。

### 17.3 实测

```text
$ triproxy core update --all --check
mihomo    up to date (1.19.33)
sing-box  up to date (1.14.4)
xray      up to date (26.10.31)

# 降级后
$ triproxy core install mihomo --version 1.19.20 --force
$ triproxy core update mihomo --check
mihomo    1.19.20 → 1.19.33
$ triproxy core update mihomo
mihomo    1.19.20 → 1.19.33
 INFO downloading core binary tag=v1.19.33 asset=mihomo-windows-amd64-compatible-v1.19.33.zip
 WARN download attempt failed ... （直连失败，镜像回退成功）
          installed 1.19.33
$ triproxy core update mihomo --check
mihomo    up to date (1.19.33)
```

那条 WARN 是镜像回退机制在工作，不是错误。

### 17.4 没有 GUI 入口

GUI 目前没有核心管理界面（安装/卸载/更新都不在），只加一个「更新」按钮会显得孤立。
核心管理仍以 CLI 为主，等 GUI 有完整核心页时再一起做。

## 18. Phase 10 — 控制器 API（连接 / 流量）

### 18.1 一套 API，两个核心，一个例外

Mihomo 与 sing-box 都实现 Clash 兼容的 HTTP API（Mihomo 叫 `external-controller`，
sing-box 叫 `clash_api`），端口都是我们生成的配置里那个 `api_port`。

**Xray 没有等价物**——它用自家的 gRPC API，形态不同、面更大。所以基于这套 API 的功能
（连接列表、流量、真实代理延迟）**只在两个核心下可用**。界面必须说清楚，
而不是显示一个空面板让人以为是 bug。连接页底部就写着这句。

### 18.2 实测发现的两个真实细节

**空连接时返回 `null` 而不是 `[]`**：

```json
{"downloadTotal":0,"uploadTotal":0,"connections":null,"memory":0}
```

`Vec` 直接反序列化会失败，让一个空闲的核心看起来像坏了。用
`deserialize_with = "null_as_empty"` 把 null 当成空列表。

**`/traffic` 是流式端点**，每秒推一个 JSON 对象、永不关闭。取一次采样意味着读第一个
完整对象然后断开连接，而不是让调用方拿着一条它没要求的流。

### 18.3 两种延迟，不能互相冒充

| 指标 | 做法 | 证明什么 | 前提 |
|---|---|---|---|
| TCP 延迟 | 到节点 `server:port` 的握手 | 端点可达、有多远 | 无 |
| 真实延迟 | 核心通过节点发 HTTP 请求 | **代理真的能用** | 运行中的 Mihomo / sing-box |

凭据错、UUID 过期、加密不匹配——TCP 握手全都会成功，真实延迟全都会失败。
所以两个都要有，且界面上分开命名（「TCP 延迟」/「真实延迟」），不合并成一个「延迟」数字。

```bash
triproxy profile test              # TCP
triproxy profile test --http       # 真实延迟，需要核心在跑
triproxy profile test --http --url http://cp.cloudflare.com/generate_204
```

没有核心时 `--http` 会直接报错，而不是逐个节点打印同样的失败：

```text
error: proxy delay needs a running Mihomo or sing-box with its controller API
       (controller API not reachable on http://127.0.0.1:9090: ...)
```

有核心时，每个节点的错误来自核心本身并被原样带回：

```text
1  Singapore gRPC  sg.example.com:443  -  delay probe for `Singapore gRPC` failed
                                          (HTTP 503): {"message":"An error occurred in the delay test"}
```
### 18.4 实测

```text
$ curl http://127.0.0.1:9090/version
{"meta":true,"version":"v1.19.33"}

$ triproxy connections
no live connections

$ triproxy traffic
↑ 0 B /s    ↓ 0 B /s
```

连接与流量为 0 是真实的：配置里的节点是假地址，请求在解析阶段就失败了。
API 链路本身已被验证——三个端点都返回了可解析的数据，包括上面那个 `null` 的边界情况。

## 19. 应用自身更新

### 19.1 一个必须说清楚的前提

**v0.2.2 及更早的版本里没有更新代码**，所以它们永远收不到自动更新。
从包含这一节的版本开始，之后的升级才能走自动更新；这一次仍要手动装。

这不是设计缺陷，是任何自更新机制的第一次上线都要面对的事实。

### 19.2 签名不是可选项

Tauri 的更新器会校验载荷的签名。没有 `.sig` 文件，客户端**拒绝安装**——
而且是发布页看起来完全正常、用户点了「下载并安装」才失败的那种失败。

所以 `latest-json.mjs` 在签名缺失或过短时直接让发布失败，而不是生成一条
客户端会拒绝的条目。

私钥放在仓库密钥 `TAURI_SIGNING_PRIVATE_KEY`，公钥写在 `tauri.conf.json` 的
`plugins.updater.pubkey`。**私钥丢失意味着所有已安装的副本再也无法自动更新**，
需要用户手动重装。

### 19.3 只发布 lite 作为更新载荷

lite 与 full 的应用二进制完全相同，差别只是 `full` 的安装包额外携带 `cores/`。

更新器替换的是应用本身：NSIS 安装包会覆盖它携带的文件，不会删除目录里其他东西。
所以 `full` 用户用 lite 包更新后，`cores/` 仍在；而如果发布 `full` 作为载荷，
每次更新都要多下载约 45 MB 换一个用户已经有的东西。

代价是这个假设依赖 NSIS 的覆盖语义——没有实测过 full → lite 的更新路径。

### 19.4 手动触发，不自动检查

每次启动都检查更新会多一次网络往返，并在用户做别的事情时打断他。
设置页一个「检查更新」按钮就是全部功能。

## 20. Clash YAML 导入

### 20.1 只导入 proxies 段

只取 `proxies:` 列表。**proxy-groups 与 rules 一律忽略**——
本程序自己生成单一的 `PROXY` 组和自己的分流规则，导入 Clash 的 rules 等于重新实现它的规则引擎，
那是另一个产品。

### 20.2 不支持的条目要报出来，不能悄悄丢掉

一份 60 节点的配置导入后只剩 48 个，用户必须知道另外 12 个去哪了：

```text
$ triproxy profile check clash.yaml
clash-test: 3 nodes, 0 errors, 1 warnings

  warning clash.unsupported_proxy  `Hy2 Unsupported`: proxy type `hysteria2` is not supported
```

同样的原则适用于**未知传输**（`network: quic`）和**未知 vless flow**：
猜一个值会改变流量如何封装，比直接拒绝更糟。

### 20.3 两个容易搞错的默认值

- **trojan 没写 `tls` 也按 TLS 处理**。Clash 配置里常省略这个键，但 trojan 明文是没有意义的
- **ss 的 `plugin` 会重建回 SIP002 字符串**（`obfs-local;obfs=http;obfs-host=x`）。
  统一成一种表示，三个编译器就不用各写一套分支

### 20.4 验证

导入一份含 reality / ws+tls / ss / hysteria2 的配置后，三个核心全部接受编译结果：

```text
mihomo    ok
sing-box  ok
xray      ok
```

## 21. QUIC 协议（Hysteria2 / TUIC）

### 21.1 第一批「只有两个核心能用」的协议

之前所有节点三个核心都能跑，能力门禁（硬约束淘汰）从未真正触发过。
Hysteria2 与 TUIC 都基于 QUIC，**Xray 完全不支持**——门禁第一次有了实际意义。

混合配置时理由会逐条列出：

```text
smart: selected `sing-box` — ...; eliminated: xray (protocol tuic, protocol hysteria2)
```

单个协议时：

```text
$ triproxy start auto
using active profile `hy2` (1 nodes)
smart: selected `sing-box` — sing-box is the default general-purpose core;
  no candidate needed a specialisation; eliminated: xray (protocol hysteria2)
```

强制指定 xray 时直接拒绝，而不是生成一份悄悄丢掉节点的配置：

```text
$ triproxy profile compile hy2.txt --core xray --check
error: `xray` cannot serve this profile: protocol hysteria2
```

### 21.2 TLS 不是选项

Hysteria2 没有明文模式。链接格式里也没有 `security` 参数，所以 TLS 是**构造出来的**，
不是从参数推导的；Clash 配置里省略 `tls` 键同样按 TLS 处理。

### 21.3 混淆参数成对出现

`obfs` 和 `obfs-password` 缺一不可。只给 obfs 的链接在导入阶段就拒绝——
服务端本来也会拒绝，但让它在握手阶段失败，用户只会看到「连不上」。

目前只定义 `salamander` 一种；其他值在校验阶段给警告。

### 21.4 两个核心的字段名不一样

Mihomo 用连字符，sing-box 嵌套成对象：

```yaml
# mihomo
type: hysteria2
password: secret
obfs: salamander
obfs-password: obfspw
tls: true
sni: www.bing.com
```

```json
// sing-box
{ "type": "hysteria2", "password": "secret",
  "obfs": { "type": "salamander", "password": "obfspw" },
  "tls": { "enabled": true, "server_name": "www.bing.com" } }
```

两种都被各自的核心校验通过（`mihomo -t` / `sing-box check`）。

### 21.5 TUIC 与 Hysteria2 的三处不同

| 维度 | Hysteria2 | TUIC |
|---|---|---|
| 凭据 | 单个 `password` | `uuid:password`，userinfo 里用冒号分隔 |
| 混淆 | `obfs` + `obfs-password` | 无 |
| 默认 ALPN | 无要求 | `h3`，链接没写也要补上 |

最后一条容易漏：QUIC 需要 h3，缺了它握手会失败，而失败信息不会提到 ALPN。
所以解析阶段就把 `h3` 作为默认值填进去，而不是指望核心补。

TUIC 的拥塞控制（`bbr` / `cubic` / `new_reno`）和 UDP 中继模式（`native` / `quic`）
都是可选字段，未知值在校验阶段给警告——它们不影响能否连接，只影响传输表现。

## 22. 尚未实现（明确的边界）

这一节按「缺了会影响什么」排序，而不是按规划书章节。**状态会过时，改动功能时请同步更新。**

### 22.1 影响日常使用

**切换节点没有界面。** 生成的配置里 `PROXY` 组是 `type: select`，成员是全部节点，
Mihomo 与 sing-box 的控制器 API 支持运行时切换（`PUT /proxies/PROXY`），**能力本来就有**——
只是我们没做这个调用，所以用户只能改配置再重启。Xray 没有控制器 API，切换确实需要重启。
这是「节点页只能看不能选」的根因，也是目前性价比最高的一个缺口。

**自定义分流规则。** 只有 `global` / `split` 两个预设，不能加自己的域名/IP 规则，
也不能管理 rule-set 订阅。

**订阅没有后台定时刷新。** 自动刷新只在显式执行 `triproxy profile refresh` 时发生。

### 22.2 影响完整性

| 缺口 | 说明 |
|---|---|
| 协议 | VLESS / VMess / Trojan / Shadowsocks / Hysteria2 / TUIC。SOCKS、HTTP、WireGuard、SSH、AnyTLS 都没有 |
| 订阅格式 | 支持 URI 列表、base64 与 Clash YAML 的 proxies 段；**不导入 proxy-groups 与 rules** |
| 流量历史 | 只有瞬时采样，没有曲线图；数据不落盘 |
| 节点管理 | 没有收藏、分组、手动排序 |
| 应用更新 | 核心与应用都能更新，但应用更新只在包含该功能的版本之后生效 |
| 多语言 | 界面只有中文 |

### 22.3 只在部分平台成立

- **系统代理**：Windows 实测过；macOS（`networksetup`）与 Linux（`gsettings`）代码写好但未真机验证
- **连接 / 流量 / 真实延迟**：依赖控制器 API，Xray 没有，界面上会说明
- **macOS / Linux 安装包**：v0.2.2 起提供，构建与产物内容经 CI 校验，但没有在真机上装过

### 22.4 明确不做

- **规则分流做成 Clash 那样的规则编辑器**：与「智能选核」的定位冲突，且维护成本远超收益
- **内置节点分享 / 订阅分发**：与软件定位无关
- **TUN 之外的流量劫持手段**（如 WFP 驱动）：TUN 已经够用，驱动层风险不值得
