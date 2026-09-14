# Niko 开发约定

## 改动后必须双端验证（强制）

每次代码改动完成后，主动运行（不需要用户提醒）：

```bash
scripts/dual-verify.sh
```

- 自动在 Mac 本机 + 远端 Windows 服务器（Server Deck 主机 `win`）上各跑一遍 tsc + vite build + 全部 node 测试，最后输出双端 PASS/FAIL 汇总。
- 任一端 FAIL 必须先修复再交付，不允许只看 Mac 通过就算完。
- 只改了前端/TS：`--win-only` 或 `--mac-only` 可加速定位，但交付前仍需完整双端 PASS。
- Rust（src-tauri）改动：本地 Win 因 Smart App Control 无法跑 cargo（os error 4551，勿再尝试），Rust 编译验证由 CI 的 `rust-check` 矩阵（macos-latest + windows-latest）在 push 后覆盖；本地无需跑 cargo。

## 同步与垃圾控制

- Win 端验证目录 `C:\Users\xzb17\niko-verify` 每次同步前清空重建，不留旧文件残留。
- 同步用的 tarball（本地 /tmp 与远端 C:\Users\xzb17）用完即删，脚本已内置，勿移除。
- 不要在 Win 上留存 cargo target、安装包等大体积中间产物。
