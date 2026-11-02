<!--
  Copyright (C) 2026 luoqingci
  SPDX-License-Identifier: GPL-3.0-or-later
  Part of TriProxy. See LICENSE and ADDITIONAL-TERMS.md.
-->

## 本次更新（v0.2.4）

**关掉窗口不再断网。** 代理客户端的窗口本来就只是面板，现在它终于只是面板了。

### 托盘驻留

- 点窗口关闭按钮 = 收进托盘，核心继续跑，系统代理一动不动；
- 左键点托盘图标显示或收起窗口，右键出菜单：显示主窗口 / 启动·停止核心 / 退出；
- 托盘菜单「退出」才会真的结束进程，并且**先停核心、再还原系统代理**——否则用户以为已经关掉，
  流量还在偷偷走代理；
- 不想驻留可以在设置里关掉「关闭窗口时驻留托盘」，退回原来的行为。

托盘创建失败时（例如没有 StatusNotifier 的 Linux 会话）不会硬撑：关闭窗口按普通退出处理，
不会出现「窗口没了、进程还在、找不回来」。

### 日志不再谎报 WARN

之前的日志面板会被 `WARN ... inbound connection from 127.0.0.1` 刷屏，看着像核心在疯狂报错。
三个问题叠在一起：

- 级别推断对 stderr 上的内容一律兜底成 WARN，而三个核心的普通日志**全部**走 stderr；
- 核心即使输出到管道也强制上色，`ESC[36mINFO ESC[0m` 既显示成 `[36m` 乱码，又把级别词切碎；
- 那些连接确实存在——界面每 2 秒轮询一次状态，每次都真的去连本地入站端口，核心就记一行。

现在级别读核心自己声明的字段（sing-box `INFO`、mihomo `level=info`、xray `[Info]`），
色码在入库前剥掉，健康探测结果复用 10 秒。逐连接记录在日志页默认折叠，右上角显示折叠了多少条。

### 第一次真正验证自动更新

v0.2.3 是第一个带更新功能的版本，但没有「上一版」可升。这一版是第一次可以从上一版直接升级：
**如果你装的是 v0.2.3，设置页点「检查更新」应该能直接升到 v0.2.4**，不用再下载安装包。

### 修复：AppImage 的签名文件覆盖安装包

发布流水线里 `*.AppImage*` 这个通配把 `.AppImage.sig` 也算进产物，签名文件因此覆盖了真正的
AppImage，Linux 用户拿到的是 400 字节的签名而不是 90MB 的安装包。

### v0.2.0 起的功能

从「能管核心」到「能当客户端用」。v0.1.0 只有核心生命周期管理，此后补齐了订阅、配置、选核、GUI、系统代理、TUN、DNS 与分流。

**订阅与配置**

- 分享链接解析：VLESS / VMess / Trojan / Shadowsocks / Hysteria2 / TUIC，支持 URI 列表、base64 订阅与 Clash 配置
- URL 订阅：条件请求缓存（`ETag` / `Last-Modified`），刷新失败不会毁掉已有节点
- 同一份 Unified Profile 编译成三个核心各自的配置，且**由核心自己校验**后才使用

**智能选核**

- 硬约束淘汰 → 原生特性数量 → 文档化默认顺序，同一份配置永远选出同一个核心
- 决策可解释：给出理由，并列出被淘汰的核心及原因

**桌面客户端**

- 仪表盘 / 节点 / 连接 / 配置 / 日志 / 设置
- 实时流量、连接列表与逐个关闭、延迟测试、订阅刷新

**网络接管**

- 系统代理：先落盘快照再改系统设置，崩溃后下次启动自愈
- TUN：管理员实测，硬杀进程后网卡与路由均被清理
- DNS：`off` / `split` / `fakeip` 三种模式，被代理的域名在隧道内解析
- 分流：`global` 全部走代理，`split` 局域网与国内目标直连

**诊断与维护**

- 两种延迟分开命名：`TCP 延迟` 只说明端点可达，`真实延迟` 由核心通过节点发请求、能证明代理可用
- `core update` 检查并安装核心更新

## 下载哪一个？

每个平台都提供桌面安装包，两个变体：

| 变体 | 内容 | 适用 |
|---|---|---|
| `*-lite` | 图形界面 + 命令行，不含核心 | 首次运行会联网下载核心 |
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
- **系统代理只在 Windows 上实测过**，macOS（`networksetup`）与 Linux（`gsettings`）的实现已写好但未经真机验证
- **macOS 与 Linux 的桌面安装包仍未经真机验证**：构建与产物内容由 CI 校验（确认核心已打包），
  但没有实际安装过。Windows 版本是本地实测的。这两个平台如果装不上，请反馈，不要当成「应该能用」
- **订阅没有后台定时更新**，自动刷新只在显式执行 `triproxy profile refresh` 时发生
- **`split` 分流是预设规则**（局域网 + 国内直连），尚不支持自定义规则
- **托盘驻留只在 Windows 上验证过**：macOS 与 Linux 的托盘图标走同一套 Tauri 接口，但没有真机点验；
  没有托盘环境时应用会退回「关闭即退出」的行为
- 暂无节点收藏与流量历史图表

## 授权

TriProxy 以 GNU GPL v3（或更新版本）发布，并附 GPLv3 第 7 条附加条款：
分发须保留版权与许可声明；分发修改版须更换名称，不得暗示与原版同源或获得原作者支持。
详见 `LICENSE` 与 `ADDITIONAL-TERMS.md`。

随包分发的核心是独立作品，适用各自授权（Mihomo GPL-3.0、sing-box GPL-3.0-or-later、
Xray MPL-2.0），完整来源、版本与对应源码地址见包内 `cores/THIRD-PARTY-NOTICES.txt`。
