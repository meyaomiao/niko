#!/usr/bin/env bash
# dual-verify.sh — Mac 本机 + 远端 Windows 双端同步验证
# 用法: scripts/dual-verify.sh [--mac-only|--win-only]
set -euo pipefail
cd "$(dirname "$0")/.."

WIN_HOST="${NIKO_WIN_HOST:-win}"        # Server Deck ledger 里的主机标识
WIN_REMOTE_DIR="${NIKO_WIN_DIR:-C:\\Users\\xzb17\\niko-verify}"
TARBALL="/tmp/niko-sync.tar.gz"
RUN_MAC=1
RUN_WIN=1
for arg in "$@"; do
  case "$arg" in
    --mac-only) RUN_WIN=0 ;;
    --win-only) RUN_MAC=0 ;;
    *) echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

MAC_FAIL=0
WIN_FAIL=0

run_mac() {
  echo "==> [mac] local verify"
  if ! npm run build; then MAC_FAIL=1; fi        # tsc --noEmit + vite build
  for t in test:auth test:balance test:login test:model-selection test:tags test:order test:catalog test:component-order test:smoke test:registration test:sessions test:station test:update test:usage; do
    echo "-- mac test: $t"
    npm run "$t" || MAC_FAIL=1
  done
  [ "$MAC_FAIL" = 0 ] && echo "==> [mac] PASS" || echo "==> [mac] FAIL"
}

sync_to_win() {
  echo "==> [win] packing source snapshot"
  tar czf "$TARBALL" \
    --exclude=node_modules --exclude=dist --exclude=.git \
    --exclude='src-tauri/target' --exclude='src-tauri/gen' \
    --exclude='src-tauri/WixTools' --exclude=release-artifacts \
    --exclude=website --exclude=.DS_Store \
    .
  echo "==> [win] uploading ($(du -h "$TARBALL" | cut -f1))"
  scp -q "$TARBALL" "${WIN_HOST}:C:/Users/xzb17/niko-sync.tar.gz"
  ssh "$WIN_HOST" "cmd.exe /c \"rmdir /s /q $WIN_REMOTE_DIR 2>nul & mkdir $WIN_REMOTE_DIR && cd /d $WIN_REMOTE_DIR && tar -xzf C:\\Users\\xzb17\\niko-sync.tar.gz && del C:\\Users\\xzb17\\niko-sync.tar.gz\""
  rm -f "$TARBALL"
}

run_win() {
  sync_to_win
  echo "==> [win] verify on ${WIN_HOST}"
  local remote="$WIN_REMOTE_DIR"
  # 每次同步会清空目录，node_modules 需要重装（npm ci 实测约 3 秒）
  local cmds=(
    "cd /d $remote && npm ci --no-audit --no-fund"
    "cd /d $remote && npm run build"
  )
  for t in test:auth test:balance test:login test:model-selection test:tags test:order test:catalog test:component-order test:smoke test:registration test:sessions test:station test:update test:usage; do
    cmds+=("cd /d $remote && npm run $t")
  done
  # Rust 层不在此验证：Win 系统的 Smart App Control 会拦截 cargo 编译产物（os error 4551）。
  # Rust 编译由 CI rust-check 矩阵（macos-latest + windows-latest）在每次 push 时覆盖。
  local c
  for c in "${cmds[@]}"; do
    echo "-- win: ${c#*/d $remote && }"
    if ! ssh "$WIN_HOST" "cmd.exe /c \"$c\""; then WIN_FAIL=1; fi
  done
  [ "$WIN_FAIL" = 0 ] && echo "==> [win] PASS" || echo "==> [win] FAIL"
}

[ "$RUN_MAC" = 1 ] && run_mac
[ "$RUN_WIN" = 1 ] && run_win

echo
echo "===== dual-verify summary ====="
[ "$RUN_MAC" = 1 ] && { [ "$MAC_FAIL" = 0 ] && echo "mac: PASS" || echo "mac: FAIL"; }
[ "$RUN_WIN" = 1 ] && { [ "$WIN_FAIL" = 0 ] && echo "win: PASS" || echo "win: FAIL"; }
exit $(( MAC_FAIL + WIN_FAIL ))
