# GPUI 版接入 CI：需要改什么、需要你定什么

现状（`.github/workflows/build.yml`）：

```
version → build-macos(xcodebuild) → build-android → release
```

`arch-package.yml`、`publish-aur.yml` 依赖 `Build and Release` 的成功运行。

GPUI 版要接进来，**macOS 那个 job 的构建产物会换掉**，其余流程可以不动。下面是具体差异。

## 1. build-macos 的差异

| 项目 | 现在（SwiftUI） | GPUI 版 |
|---|---|---|
| 构建命令 | `xcodebuild build` | `cargo build --release -p manis-pocket-app` + `scripts/bundle-macos.sh` |
| 产物 | `ManisPocket.app` | `build/Manis Pocket.app` |
| 签名 | 现有 provisioning/certificate 流程 | **需要重新确认**：我们用的是 ad-hoc 签名，发布给用户需要 Developer ID + notarization |
| 最低系统 | 现有设置 | 13.0（`LSMinimumSystemVersion`） |
| 运行依赖 | 无 | 无（纯 Rust，静态链接 GPUI 依赖） |
| 构建时间 | — | 首次约 8–12 分钟，带 cargo 缓存后约 3–5 分钟 |

替代 job 的步骤已经写成样例：`packaging/ci/gpui-macos-build.yml.sample`（**没有放进 `.github/`，不会生效**）。

## 2. 必须由你决定的三件事

1. **是否替换现有 macOS 产物**，还是先并行发布（比如 `ManisPocket-GPUI.app` 作为附加产物）。
2. **版本号策略**：GPUI 版继承 `2.7.x` 序列，还是从 `3.0.0` 起（重写通常意味着大版本）。
3. **签名与公证**：沿用现有 Developer ID 证书 + `notarytool`，还是先发未公证的 beta（Gatekeeper 会拦截，用户需右键打开）。

这三件定下来，我就能把 workflow 改动一次做完并跑通。

## 3. 与 Linux 的关系

`arch-package.yml` 现在构建 GTK4 版 + Wayland CLI：
- `manis-pocket-linux`（GTK4 GUI）在新方案里要被 `manis-pocket-app` 取代；
- `manis-pocket-wayland`（CLI/同步引擎）保留，它提供 `manis-pocket-sync` 的命令行入口。

也就是说 AUR 包内容需要一次调整：GUI 换成 GPUI 二进制，CLI 保持。

## 4. 我建议的落地顺序

1. 先在 CI 里**只加一个不发布的构建 job**（PR 上跑 `cargo build --release` + 测试），确保持续可构建；
2. 视觉/IME 收尾完成后，再让 release job 产物指向 GPUI 版；
3. 最后再切换 AUR。

这样任何一步出问题都能回退，不会让用户手上突然收到半成品。

## 5. 需要的验证（CI 跑通之前）

- [ ] macOS：`cargo test` 全绿 + `scripts/bundle-macos.sh` 产物可启动
- [ ] macOS：菜单栏图标、快捷键、托盘菜单在签名后的 .app 里正常
- [ ] macOS：IME 候选框（**当前未确认，见下**）
- [ ] Linux：`scripts/install-linux.sh` 后能从启动器启动、自启动生效
- [ ] Linux：Wayland 剪贴板读写（`wl-paste`/`wl-copy`）在 CI 容器里无法测，只能实机验证

## 6. 已知未完成项（发布前必须解决）

| 项 | 状态 | 影响 |
|---|---|---|
| 中文输入法候选框不出现 | 已定位到 macOS 后端光标矩形，已加回退；**待实机确认** | 中文用户无法选词 |
| 删除图标是 ✕ 而非垃圾桶 | gpui-kit 图标集无 Trash | 视觉差异，非功能 |
| 行内 hover 按钮未做视觉校对 | 代码就绪 | 可能位置/大小不对 |
