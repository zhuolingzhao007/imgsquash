#!/bin/bash
# 开发宿主（等价 nsh run，但把 Rust 依赖按 opt-level=2 构建）。
#
# nsh run 默认是 debug 构建：quantizr / oxipng / SSIMULACRA2 全是没优化的，
# 一张 1000×750 的照片要几十秒；release 只要 2.8 秒/张。
#
# .nsh/rust/Cargo.toml 每次都由 nsh 重新生成，profile 写不进去，
# 只能走 cargo 的环境变量（和 package.sh 里 CARGO_PROFILE_RELEASE_STRIP 同一套机制）。
#
# 用法：./scripts/dev.sh                 → 默认 macOS 宿主
#       ./scripts/dev.sh --target macos  → 参数原样转给 nsh run
set -e
cd "$(dirname "$0")/.."

# 只影响 Rust 侧；JS 仍然是 dev server 的热重载。
export CARGO_PROFILE_DEV_OPT_LEVEL=2
exec nsh run "$@"
