#!/bin/bash
# 打包 ImgSquash（macOS）。Windows 用同目录的 package.ps1。
#
# nsh 生成的 release profile 不带 strip，符号表要占 6.7MB，
# 打包时传 CARGO_PROFILE_RELEASE_STRIP=symbols 让 cargo 剥掉：30MB → 23.8MB。
# 注意 .nsh/rust/Cargo.toml 每次打包都会被 nsh 重新生成，手改无效，
# profile 只能走这个环境变量。
set -e
cd "$(dirname "$0")/.."

# 确保 nsh.json 的 name 一致
python3 -c "
import json
path='nsh.json'
with open(path) as f:
    d=json.load(f)
d['name']='imgsquash'
with open(path,'w') as f:
    json.dump(d,f,ensure_ascii=False,indent=2)
    f.write('\n')
"

OUT_DIR=".nsh/out"
rm -rf "$OUT_DIR"/*.app

CARGO_PROFILE_RELEASE_STRIP=symbols nsh package --target macos --name 'imgsquash' --icon assets/icon.png

# nsh 0.3 起 --name 直接决定产物名，已经是 imgsquash.app 就不用动；
# 旧版本会按 crate 名生成，这里兜底改成 imgsquash.app。
APP_OUT=$(find "$OUT_DIR" -maxdepth 1 -name "*.app" -type d | head -n1)
if [[ -n "$APP_OUT" && "$APP_OUT" != "$OUT_DIR/imgsquash.app" ]]; then
  rm -rf "$OUT_DIR/imgsquash.app"
  mv "$APP_OUT" "$OUT_DIR/imgsquash.app"
  echo "已重命名为: $(basename "$APP_OUT") -> imgsquash.app"
fi

echo "打包完成："
ls -la "$OUT_DIR"/imgsquash.app
