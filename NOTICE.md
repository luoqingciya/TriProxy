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

# NOTICE — 第三方组件

TriProxy 本身不包含任何代理核心的源代码。TriProxy 只是这些核心的启动器：
负责定位、校验、启动、停止与监督，不修改其行为。

## 两种发行形态

| 形态 | 是否包含核心二进制 | 说明 |
|---|---|---|
| `lite` | 否 | 首次运行时从上游官方 Release 下载，安装前校验 SHA256 |
| `full` | 是 | CI 打包时下载并校验后放入 `cores/`，随包分发**未经修改**的上游构建 |

`full` 形态属于再分发，相关义务见下文「再分发与源码获取」。

## 三个核心

| 核心 | 上游仓库 | 授权 |
|---|---|---|
| Mihomo | [MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo) | GPL-3.0 |
| sing-box | [SagerNet/sing-box](https://github.com/SagerNet/sing-box) | GPL-3.0-or-later |
| Xray-core | [XTLS/Xray-core](https://github.com/XTLS/Xray-core) | MPL-2.0 |

授权信息以各仓库 `LICENSE` 文件为准。

## 再分发与源码获取

`full` 发行包中的三个核心均为上游官方构建，**未作任何修改**（未重新编译、未打补丁、
未改动默认配置）。TriProxy 与它们之间只通过命令行参数与进程边界交互。

GPLv3 与 MPL-2.0 对二进制再分发均要求可获取对应源码。`full` 包内附带
`cores/THIRD-PARTY-NOTICES.txt`，逐项列出每个核心的确切版本、上游仓库、
对应源码 tag 地址与发布页地址，例如：

```text
mihomo 1.19.33
  upstream: https://github.com/MetaCubeX/mihomo
  license:  GPL-3.0
  source:   https://github.com/MetaCubeX/mihomo/tree/v1.19.33
  release:  https://github.com/MetaCubeX/mihomo/releases/tag/v1.19.33
  sha256:   <安装时校验通过的值>
```

该文件由发布流水线根据实际下载的版本自动生成，与包内二进制一一对应，不会出现
「声明版本与实际不符」的情况。

## 校验和来源

- 首选：GitHub Releases API 返回的 asset `digest` 字段（`sha256:...`）。
- 回退：上游随包发布的校验文件（Xray 的 `.dgst`、部分项目的 `checksums.txt`）。
- 两者都取不到时**拒绝安装**，除非用户显式传入 `--allow-unverified`。

发布流水线走的是同一套流程，因此 CI 打包与终端用户下载受同等校验保护。

## 镜像

下载失败时可按 `--mirror` 或环境变量 `TRIPROXY_GITHUB_MIRROR` 使用第三方 GitHub 加速服务。
校验和始终来自 `api.github.com`，不来自镜像，因此镜像无法注入被替换的二进制。

## Rust 依赖

依赖清单见各 `Cargo.toml`。主要依赖：tokio、reqwest（rustls）、serde、clap、sha2、zip、
flate2、tar、tracing。均为宽松许可（MIT / Apache-2.0 / BSD），与本项目的 GPL-3.0-or-later 兼容。
