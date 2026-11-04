#!/bin/bash
# 固定实图和边界图的 release 验收；原图/输出/CSV/并排预览写到 .nsh/out/quality。
set -e
cd "$(dirname "$0")/.."
cargo test --release --lib acceptance_corpus -- --ignored --nocapture
