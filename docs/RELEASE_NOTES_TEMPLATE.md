<!--
  Copyright (C) 2026 luoqingci
  SPDX-License-Identifier: GPL-3.0-or-later
  Part of TriProxy. See LICENSE and ADDITIONAL-TERMS.md.
-->

## 本次更新（v0.2.9）

**这一版把「Linux」拆成了「发行版」。** Fedora 用户此前装不上，装上了也有两件事会让人以为软件坏了——
而且两件都不给原因。

### 新：Fedora / RHEL 系的 `.rpm`

`dnf` 装不了 `.deb`，AppImage 还要求 FUSE 2 在位，所以 Fedora 上原本没有一条正常的安装路径。现在有：

```bash
sudo dnf install ./TriProxy-0.2.9-linux-x64-lite.rpm
```

依赖（`webkit2gtk4.1`、`gtk3`、`libappindicator-gtk3`）由 `dnf` 从仓库解析，卸载走
`dnf remove triproxy`。

有一处与 deb 不同，值得写下来：**RPM 的依赖是我们手写的，不是从二进制里推出来的。** tauri 的 RPM
打包器用纯 Rust 的 `rpm` crate，不调 `rpmbuild`，也完全不读 ELF 的 `NEEDED`（deb 打包器会）。
这个差别唯一的症状是「安装时报 `nothing provides …`」——所以发布流水线加了 `rpm -qlp` 与
`rpm -qpR` 两道断言：包做出来先自己验一遍载荷和依赖，不等用户装的时候才发现。

### 新：`triproxy doctor` 报出发行版事实

「linux」不是一个平台。同一个二进制在 Fedora Workstation 上与 Ubuntu 上的现场完全不同，而这两件事
此前都不给原因。`doctor` 现在多五行：

```text
  distro     Fedora Linux 45 (fedora, rpm)
  desktop    GNOME — 托盘不可用，关闭窗口将退出
             sudo dnf install gnome-shell-extension-appindicator
  selinux    enforcing
  tun device /dev/net/tun 不存在 — 先执行 sudo modprobe tun
  proxy      linux-gsettings — 可用
```

四行里三行是「不查文档就不会知道」的：Fedora Workstation 默认不装 AppIndicator 扩展（GNOME 自己没有
托盘，图标是这个扩展提供的）、SELinux 在 enforcing、`tun` 模块没被任何东西拉起来时 `/dev/net/tun`
根本不存在。最后一行是系统代理后端自检——说不准时它说「不可用」，而不是假装能开。

发行版与包族的判断只用于**措辞**（一条命令该写 `dnf` 还是 `apt`），没有行为分支；
托盘与 SELinux 这两项也只在真知道的时候才报，猜错一个「你大概没有托盘」比不说更烦人。

### 改进：TUN 起不来时说清缺什么

原来只查权限。现在 `start --tun` 与界面上的「连接」会同时检查设备节点：缺 `/dev/net/tun` 就直接给
`sudo modprobe tun`，而不是让核心抛一段一个字都不提设备的日志。命令行与界面共用同一个判定，
不会一边说权限、一边说设备。

### 这一版新增的边界

- **KDE Plasma 的系统代理没有实现**：那里 `doctor` 会明说「不可用」。与其猜着写一段没人验证过的
  `kwriteconfig`，不如写清楚——TUN 与环境变量都还能用
- **RPM 未在真机 `dnf install` 过**：包由 CI 构建，载荷与依赖由流水线校验；三个依赖名只对
  Fedora / RHEL 系成立（openSUSE 里同一个库叫 `libwebkit2gtk-4.1-0`），所以文档也只写
  「Fedora / RHEL / Rocky」
- **Linux 的应用内自更新仍只对 AppImage 有效**：`.deb` / `.rpm` 装的那份用包管理器升级

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

**平台**

- Fedora / RHEL 系：Linux 增加 `.rpm` 包；CI 在 Fedora 容器里编译并跑测试
- `triproxy doctor` 报出发行版与包族、桌面会话与托盘宿主、SELinux 状态、`/dev/net/tun`、系统代理后端
- TUN 启动前检查设备节点：缺 `/dev/net/tun` 时直接给 `sudo modprobe tun`，而不是让核心抛日志

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
| Linux | `TriProxy-<版本>-linux-x64-{lite,full}.AppImage` / `.deb` / `.rpm` | AppImage 加执行权限直接运行；deb / rpm 用包管理器安装 |

文件名读作「平台-架构-变体」：`windows-x64-lite` 就是 Windows / 64 位 Intel / 精简版，
不用再靠扩展名反推系统。

Linux 三种格式对应三种机器：

```bash
sudo apt install ./TriProxy-<版本>-linux-x64-lite.deb    # Debian / Ubuntu
sudo dnf install ./TriProxy-<版本>-linux-x64-lite.rpm    # Fedora / RHEL / Rocky / AlmaLinux
chmod +x TriProxy-<版本>-linux-x64-lite.AppImage         # 任意发行版（需 FUSE 2）
```

Fedora 用 `.rpm`：`dnf` 装不了 `.deb`，而 AppImage 还要求 FUSE 2 在位。装完先跑一次
`triproxy doctor`——它会报出这台机器上的两件 Fedora 常见事（托盘缺 AppIndicator 扩展、
`/dev/net/tun` 不存在导致 TUN 起不来）并给出对应命令。

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
- **内存数字只在 Windows 上核对过**：三个平台都读 RSS，Linux 上确认过数字会随核心运行出现并变化，
  但没有与 `ps` 逐项比对；「应用内存」不含 WebView 的渲染进程，任务管理器里看到的总和会更大
- **下拉框的弹出列表在 Linux 上依赖引擎支持**：关闭状态的控件由我们自己绘制，任何平台都一致；
  展开后的列表仍由 GTK 画，`color-scheme` 能让它变深——Ubuntu 26.10 上正常，其他发行版的
  主题设置仍可能有差异
- **节点管理**：能搜索、按协议过滤、点选切换、收藏并只看收藏，但没有分组和手动排序
- **系统代理**：Windows 与 Linux（Ubuntu 26.10 Desktop，`gsettings`）实测可用；macOS 的
  `networksetup` 路径按同一接口实现，未经真机验证。Linux 上只驱动 GNOME，KDE Plasma 会话里
  `doctor` 会显示「不可用」
- **托盘**：Windows 与 Linux（Ubuntu 26.10 Desktop）实测正常；macOS 走同一套 Tauri 接口，没有真机点验。
  没有托盘环境时应用会退回「关闭即退出」的行为。Fedora Workstation 默认不带 AppIndicator 扩展，
  开箱状态下托盘不可用，装扩展即可（`sudo dnf install gnome-shell-extension-appindicator`）
- **Linux 的应用内自更新只对 AppImage 有效**：`.deb` / `.rpm` 装的那份用包管理器升级
  （`sudo dnf upgrade triproxy` / `sudo apt install --only-upgrade triproxy`）
- **Fedora 的 RPM 由 CI 构建并校验载荷与依赖，未在真机上装过**：Fedora 45 上核对了发行版与桌面
  探测、TUN 设备检查、`gsettings` 代理写入与还原；完整构建由 CI 的 Fedora 任务覆盖（容器里
  `cargo check` + `cargo test` + 桌面外壳编译）。如果 `dnf install` 失败，请反馈
- **深链接只在 Windows 上验证过**：macOS 依赖打包后的 Info.plist，Linux 需要 `.desktop` 注册，均未真机点验
- **开机自启在 Windows 上会弹一次 UAC**（应用清单要求管理员权限）；改用计划任务可以静默启动，尚未做
- **macOS 的桌面安装包仍未经真机验证**：构建与产物内容由 CI 校验（确认核心已打包），但没有实际安装过。
  Windows 与 Linux（`.deb` / AppImage，Ubuntu 26.10 Desktop）是实测的。macOS 如果装不上，请反馈，
  不要当成「应该能用」

## 授权

TriProxy 以 GNU GPL v3（或更新版本）发布，并附 GPLv3 第 7 条附加条款：
分发须保留版权与许可声明；分发修改版须更换名称，不得暗示与原版同源或获得原作者支持。
详见 `LICENSE` 与 `ADDITIONAL-TERMS.md`。

随包分发的核心是独立作品，适用各自授权（Mihomo GPL-3.0、sing-box GPL-3.0-or-later、
Xray MPL-2.0），完整来源、版本与对应源码地址见包内 `cores/THIRD-PARTY-NOTICES.txt`。
