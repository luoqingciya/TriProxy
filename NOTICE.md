# NOTICE — 第三方组件

TriProxy 本身不包含、也不分发任何代理核心的二进制文件。
首次使用时，程序从各项目的官方 GitHub Release 下载对应平台的构建，并在安装前校验 SHA256。

## 三个核心

| 核心 | 上游仓库 | 授权 |
|---|---|---|
| Mihomo | [MetaCubeX/mihomo](https://github.com/MetaCubeX/mihomo) | GPL-3.0 |
| sing-box | [SagerNet/sing-box](https://github.com/SagerNet/sing-box) | GPL-3.0-or-later |
| Xray-core | [XTLS/Xray-core](https://github.com/XTLS/Xray-core) | MPL-2.0 |

授权信息以各仓库 `LICENSE` 文件为准。若后续改为随安装包分发二进制，需在此处补充完整的授权声明与对应源码获取方式。

## 校验和来源

- 首选：GitHub Releases API 返回的 asset `digest` 字段（`sha256:...`）。
- 回退：上游随包发布的校验文件（Xray 的 `.dgst`、部分项目的 `checksums.txt`）。
- 两者都取不到时**拒绝安装**，除非用户显式传入 `--allow-unverified`。

## 镜像

下载失败时可按 `--mirror` 或环境变量 `TRIPROXY_GITHUB_MIRROR` 使用第三方 GitHub 加速服务。
校验和始终来自 `api.github.com`，不来自镜像，因此镜像无法注入被替换的二进制。

## Rust 依赖

依赖清单见各 `Cargo.toml`。主要依赖：tokio、reqwest（rustls）、serde、clap、sha2、zip、flate2、tar、tracing。
