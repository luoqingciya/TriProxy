<!--
  Copyright (C) 2026 luoqingci
  SPDX-License-Identifier: GPL-3.0-or-later
  Part of TriProxy. See LICENSE and ADDITIONAL-TERMS.md.
-->

## 本次更新（v0.2.7）

**这一版修掉一个「看起来一直很好」的假数字。** 仪表盘那行摘要里的延迟，从 v0.1 起就写着 `0 ms`——
每一次连接、每一个节点、每一台机器。它从来没测过你的节点。

### 那个 0 ms 是什么

它测的是**对本地入站端口的一次 TCP 握手**。本机 socket 往返永远在一毫秒以内，取整恒为 0。
也就是说，它只回答「端口在不在监听」，而这个问题界面上早就用另一个字段回答了。

现在显示的是真实延迟：核心发一次 HTTP 请求穿过当前节点，往返多少就是多少。

```text
sing-box · 127.0.0.1:7890 · 延迟 140 ms · 内存 86.4 MB · 已运行 5 分钟 · ↑ 12.3 KB/s · ↓ 1.2 MB/s
```

- **每 15 秒测一次**，不是跟着 2 秒的状态轮询：每次测量都是一次真实请求，两秒一次会把状态
  指示器变成流量源，还会污染旁边那张流量曲线。
- **只在仪表盘可见时测**：离开页面就停表。
- **测不到就写 `延迟 —`**，鼠标悬停能看到原因（Xray 没有控制器 API、核心还没选定节点、探测超时）。
  用 `0` 冒充一个没测到的数字，正是这一版要修掉的事。

命令行同步：`triproxy delay` 打印当前节点的真实延迟，测不到时退出码 1。`triproxy status` 里
那一列改名 `LOOPBACK`，显示 `<1 ms`——名字写清楚它是本地的，就不会有人再把它当节点延迟读。

### 新增：内存占用

- 仪表盘摘要与核心表显示**核心进程的 RSS**（就是任务管理器里那一列）
- 设置 → 高级 → 应用内存：承载界面的进程，不含 WebView 渲染进程（界面上写明了）

数字每两秒重读一次，不缓存：核心的占用随连接数和加载的规则数据变化，缓存下来等于永远显示
启动那一刻的数字。

### 修复：相对路径的数据目录会静默吃掉配置

`triproxy --data-dir ./data start` 以前是一个「看起来能用」的选项：配置写到了正确位置，
但核心启动后用的是它自己的默认配置——你导入的节点一个都不在，控制器端口也没开，**没有任何报错**。

原因是核心以 `-f <config> -d <config 所在目录>` 启动，`-d` 会改变工作目录，而 `-f` 给的是相对路径，
于是它去找 `configs/mihomo/configs/mihomo/config.yaml`，找不到就写一份自己的初始配置继续跑。
现在数据目录在解析时就被锚定为绝对路径。

### 文档

README 重写：目录、三种延迟的对照表（回环 / TCP / 真实）、内存语义、完整命令表、两个踩坑、
已知限制。架构文档新增三节，讲清这个数字的来历、内存为什么用 RSS、以及上面那个路径 bug。

### v0.2.0 起的功能

从「能管核心」到「能当客户端用」。v0.1.0 只有核心生命周期管理，此后补齐了订阅、配置、选核、GUI、系统代理、TUN、DNS 与分流。

**订阅与配置**

- 分享链接解析：VLESS / VMess / Trojan / Shadowsocks / Hysteria2 / TUIC / SOCKS5 / HTTP，支持 URI 列表、base64 订阅与 Clash 配置
- URL 订阅：条件请求缓存（`ETag` / `Last-Modified`），刷新失败不会毁掉已有节点，桌面端后台按间隔刷新
- 同一份 Unified Profile 编译成三个核心各自的配置，且**由核心自己校验**后才使用

**连接与分流**

- 代理组：`AUTO`（url-test）+ `PROXY`（select），没选过节点就是自动选择
- 自定义规则（Clash 语法，一个解析器喂三个核心）与规则集订阅（按核心格式分流）
- 系统代理：绕过列表可配，崩溃后自愈，停止时自动还原
- TUN 与 DNS：三种模式，防泄漏靠让查询本身走隧道

**桌面端**

- 托盘驻留（关窗口不断网）、单实例、`triproxy://` 一键导入订阅
- 连接列表带进程与命中规则；流量曲线；节点收藏
- 配置备份 / 恢复（合并而非覆盖）、应用自更新

## 下载哪一个？

每个平台都提供桌面安装包，两个变体：

| 变体 | 内容 | 适用 |
|---|---|---|
| `*-lite` | 图形界面 + 命令行，不含核心 | 首次运行在界面里点「下载核心」，或自行管理核心版本 |
| `*-full` | 同上 + `cores/` 内三个已校验的核心 | 开箱即用，首次运行无需联网 |

| 平台 | 文件 | 安装方式 |
|---|---|---|
| Windows | `TriProxy-<版本>-windows-x64-{lite,full}.exe` | 双击安装到 `C:\Program Files\TriProxy` |
| macOS (Apple Silicon) | `TriProxy-<版本>-macos-arm64-{lite,full}.dmg` | 打开后拖进「应用程序」 |
| Linux | `TriProxy-<版本>-linux-x64-{lite,full}.AppImage` / `.deb` | AppImage 加执行权限直接运行；deb 用包管理器安装 |

文件名读作「平台-架构-变体」：`windows-x64-lite` 就是 Windows / 64 位 Intel / 精简版，
不用再靠扩展名反推系统。

**命令行都在同一个包里**，与图形界面共用同一份数据，不需要单独下载。

### 数据放在哪里

| 平台 | 位置 | 说明 |
|---|---|---|
| Windows | `<安装目录>\data` | 整个应用一个文件夹；卸载会一并删除 |
| macOS | `~/Library/Application Support/TriProxy` | `.app` 是密封包，不应写入 |
| Linux | `~/.local/share/TriProxy` | 遵循 XDG 约定 |

Windows 上首次启动会把旧版 `%APPDATA%\TriProxy` 的数据自动迁移过来。

> 安装包未做代码签名：
> - Windows SmartScreen 提示「未知发布者」→ 点「更多信息 → 仍要运行」
> - macOS 首次打开提示「无法验证开发者」→ 右键点图标选「打开」

`full` 版的核心由 CI 通过 TriProxy 自身的下载与校验流程获取，SHA256 与上游 Release
一致，未经修改。程序优先使用用户自行安装的更新版本（`triproxy core install`），
其次才用随包的 `cores/`。

随包核心是独立作品，许可文本在 `cores/LICENSE-<core>.txt`，版本、来源与对应源码地址
见 `cores/THIRD-PARTY-NOTICES.txt`。

## 首次运行

```bash
triproxy doctor      # 环境自检
triproxy profile add https://example.com/sub --name my-sub --use
triproxy start mihomo --proxy
triproxy status
```

lite 版安装核心：

```bash
triproxy core install mihomo
triproxy core install sing-box
triproxy core install xray
```

中国大陆网络直连 GitHub Release 通常不可用，可用镜像或本地文件：

```bash
triproxy core install mihomo --mirror https://ghfast.top/
triproxy core install xray --file ./Xray-windows-64.zip
```

桌面客户端源码在同一仓库，`npm install && npm run tauri build` 构建。

## 已知限制

这些是刻意留在这一版之外的，不是遗漏：

- **Xray 没有控制器 API**，因此「连接」「流量」「真实延迟」「运行时切换节点」在 Xray 下不可用；界面会说明原因
- **Xray 不支持 Hysteria2 与 TUIC**，含这两类节点的配置无法用 Xray 运行，智能选核会自动避开
- **Xray 没有代理组**，只能选单个节点，没有「自动选择」
- **协议**：支持 VLESS / VMess / Trojan / Shadowsocks / Hysteria2 / TUIC，
  以及 SOCKS5 / HTTP 出站。仍缺 WireGuard、SSH、SSR、Snell、AnyTLS
- **规则集只能按核心格式订阅**：`.mrs`/`.yaml` 归 Mihomo，`.srs` 归 sing-box，Xray 不支持；
  一条 URL 喂不了两个核心，因为两种格式不通用
- **流量曲线不落盘**：只有最近 3 分钟，关掉窗口就没了
- **内存数字只在 Windows 上核对过**：三个平台都读 RSS，但 macOS 与 Linux 的数值没有真机对比；
  「应用内存」不含 WebView 的渲染进程，任务管理器里看到的总和会更大
- **节点管理**：能搜索、按协议过滤、点选切换、收藏并只看收藏，但没有分组和手动排序
- **系统代理只在 Windows 上实测过**，macOS（`networksetup`）与 Linux（`gsettings`）的实现已写好但未经真机验证
- **托盘与深链接只在 Windows 上验证过**：macOS 与 Linux 走同一套 Tauri 接口，但没有真机点验；
  没有托盘环境时应用会退回「关闭即退出」的行为
- **开机自启在 Windows 上会弹一次 UAC**（应用清单要求管理员权限）；改用计划任务可以静默启动，尚未做
- **macOS 与 Linux 的桌面安装包仍未经真机验证**：构建与产物内容由 CI 校验（确认核心已打包），
  但没有实际安装过。Windows 版本是本地实测的。这两个平台如果装不上，请反馈，不要当成「应该能用」

## 授权

TriProxy 以 GNU GPL v3（或更新版本）发布，并附 GPLv3 第 7 条附加条款：
分发须保留版权与许可声明；分发修改版须更换名称，不得暗示与原版同源或获得原作者支持。
详见 `LICENSE` 与 `ADDITIONAL-TERMS.md`。

随包分发的核心是独立作品，适用各自授权（Mihomo GPL-3.0、sing-box GPL-3.0-or-later、
Xray MPL-2.0），完整来源、版本与对应源码地址见包内 `cores/THIRD-PARTY-NOTICES.txt`。
