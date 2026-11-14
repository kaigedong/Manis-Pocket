# 使用

Manis Pocket 是同一个 GPUI（Rust）程序，macOS 和 Linux 共用一套代码。

## 打开与操作

- `⇧⌘C` 打开历史面板（默认在菜单栏图标下方，可在设置里改成屏幕中央）。
- 输入关键词搜索；`↑`/`↓` 选择，两端循环（在第 1 条按 `↑` 会跳到最后一条）。
- `Return` 复制选中条目；`Esc` 关闭面板。
- `⌘` 单击多条可以加入粘贴栈，条目右侧显示编号，`Return` 按选择顺序合并。
- `⌥P` 固定或取消固定；固定项排在列表顶部，与历史之间有一条分隔线。
- 鼠标悬停在行上时，右侧出现固定和删除两个图标按钮。
- 面板底部是命令行：偏好设置、关于、退出（设置里可关闭“显示底部信息栏”）。
  “关于”会打开一个小窗口，显示当前版本、运行平台和项目主页。

## 数据位置

| 平台 | 路径 |
|---|---|
| macOS | `~/Library/Application Support/ManisPocket/manispocket.db`，设置在同目录 `settings.json` |
| Linux | `$XDG_DATA_HOME/manis-pocket/manispocket.db`（通常是 `~/.local/share/manis-pocket/`） |

首次启动时，如果新库还是空的，会尝试导入旧版本留下的历史：

- macOS：SwiftUI 版的 SwiftData 存储（`~/Library/Application Support/ManisPocket/default.store`）。
- Linux：GTK 版的 `history.json`（与新库同目录），导入成功后改名为 `history.json.imported`，
  这样以后清空历史不会又被它填满。

导入失败只记录原因，不会阻止启动。

## 设备同步

两端在设置里启用同步后，会通过 mDNS 在同一局域网内互相发现，默认端口 `31774`。

1. 在 **Devices** 里找到对方设备，点击 **Pair**。
2. 两端显示同一个六位码，核对后在两端确认。
3. 配对完成后，任一端复制文本，另一端的系统剪贴板会更新。

同步只传输纯文本和 PNG 图片（单条上限 20 MiB）；文件和历史记录仍只在本机管理。

## 从仓库构建

```sh
cargo build --release -p manis-pocket-app
```

macOS 打包：

```sh
bash scripts/bundle-macos.sh
```

Linux 需要 `wl-clipboard`（提供 `wl-copy`/`wl-paste`）、Wayland、XKB 和 Fontconfig。
