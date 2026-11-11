# Manis Pocket macOS UI / 交互 1:1 规格（GPUI 重写用）

> **用途**：不读 Swift 源码，仅凭本文即可写出等价 UI。
> **来源**：`ManisPocket/`（SwiftUI + AppKit，macOS 端），所有结论后括号标注 `文件:行号`。
> **单位**：全部为 macOS 逻辑点（pt）。GPUI 的 `px()` 是逻辑像素，在 macOS 上 1px = 1pt（Retina 由 scale factor 处理），因此文中数值可直接写成 `px(22.)`。
> **基准**：macOS 14/15 分支的数值为主；仅 macOS 26 才不同的常量在文中单独标注 `[26]`。

## 0. 阅读说明

### 0.1 常量总入口

所有弹出面板的布局常量集中在 `Popup` 类，移植时应原样保留为一个 `PopupMetrics` 结构体：

| 常量 | 值 | 说明 | 来源 |
|---|---|---|---|
| `Popup.verticalSeparatorPadding` | 6.0 | 分隔线上/下内边距、列表上下 padding | `ManisPocket/Observables/Popup.swift:20` |
| `Popup.horizontalSeparatorPadding` | 6.0 | 分隔线左/右内边距 | `ManisPocket/Observables/Popup.swift:21` |
| `Popup.verticalPadding` | 5 | 窗口顶部/底部内边距 | `ManisPocket/Observables/Popup.swift:22` |
| `Popup.horizontalPadding` | 5 | 列表区左/右内边距、搜索框外距 | `ManisPocket/Observables/Popup.swift:23` |
| `Popup.minimumPreviewHeight` | 150 | 预览打开时的最小内容高度 | `ManisPocket/Observables/Popup.swift:24` |
| `Popup.cornerRadius` | 4；`[26]` 7 | 行/搜索框/内嵌块圆角 | `ManisPocket/Observables/Popup.swift:28-32` |
| `Popup.itemHeight` | 22；`[26]` 24 | 列表行最小高度 | `ManisPocket/Observables/Popup.swift:34-38` |
| `SettingsLayout.contentWidth` | 550.0 | 设置页内容区宽度 | `ManisPocket/Extensions/Settings.PaneIdentifier+Panes.swift:4` |
| `SettingsLayout.paneWidth` | 610.0 | 设置 pane 总宽（contentWidth+60） | `ManisPocket/Extensions/Settings.PaneIdentifier+Panes.swift:5` |

### 0.2 颜色语义

源工程大量使用 SwiftUI 语义色，移植时必须映射到 GPUI 的等价物，不要硬编码 RGB：

| Swift / AppKit | 语义 | GPUI 侧建议 |
|---|---|---|
| `.primary` | 主文本 | 主题 `text` 色 |
| `.secondary` | 次文本/占位 | `text_muted`（≈ 60% 前景） |
| `.tertiary` | 三级文本 | 更淡（≈ 30%） |
| `Color.accentColor` | 系统强调色（用户可改） | 读取系统 accent，GPUI 用自定义 theme 变量 |
| `NSColor.secondaryLabelColor` | 同 `.secondary` | 同上 |
| `.findHighlightColor` | 搜索命中底色（系统黄） | 固定 `#FFFF00` 近似（macOS 约 `#FFF176`） |
| `Color.secondary.opacity(0.1)` | 搜索框底 | 计算 alpha |
| `.white.opacity(0.001)` | 仅为命中测试的透明底（macOS 26 悬停 bug 规避） | GPUI 不需要，直接省略 |
| `NSVisualEffectView.Material.popover` | 弹窗毛玻璃 | GPUI `WindowBackgroundAppearance::Blurred`（近似） |
| `NSGlassEffectView.Style.regular` | macOS 26 液态玻璃 | GPUI 无，降级为 Blurred |

（来源：`ManisPocket/Views/VisualEffectView.swift:6-7,23`；`ManisPocket/Views/ListItemView.swift:115`；`ManisPocket/Observables/HistoryItemDecorator.swift:223`）

### 0.3 字体

| 用途 | 字体 | 来源 |
|---|---|---|
| 默认正文（列表标题、footer 文本、搜索框） | 系统正文字体，未显式设置 → macOS 为 13pt | `ManisPocket/Views/ListItemTitleView.swift:9`、`ManisPocket/Views/FooterItemView.swift:10` |
| 复制成功气泡、多选序号徽标 | `.caption` ≈ 10pt | `ManisPocket/Views/HistoryItemView.swift:110`、`ManisPocket/Views/ListItemView.swift:86` |
| 行内 hover 图标按钮 | `.system(size: 11, weight: .semibold)` | `ManisPocket/Views/ActionIconButton.swift:13` |
| 设备筛选 emoji | `.system(size: 14)` | `ManisPocket/Views/DeviceFilterIcon.swift:12` |
| 设置页 section 标签 | `.system(size: 13.0)` | `build/SourcePackages/checkouts/Settings/Sources/Settings/Section.swift:99` |
| 设置页说明文字（`.controlSize(.small)` + gray） | 11pt | `build/SourcePackages/checkouts/Settings/Sources/Settings/Pane.swift:92` |
| Advanced 页脚 shell 脚本 | `.system(size: 11, design: .monospaced)` | `ManisPocket/Settings/AdvancedSettingsPane.swift:17,27` |
| 配对 PIN 数字 | `.system(.title, design: .monospaced)` | `ManisPocket/Settings/SyncSettingsPane.swift:225` |
| 编辑设备弹窗标题 / 配对标题 | `.headline` | `ManisPocket/Settings/SyncSettingsPane.swift:194,219` |

GPUI 对应：`Text::new(..).text_size(px(13.))`，字体族用系统字体（macOS 上 GPUI 默认即 `.AppleSystemUIFont`）。

---

## 1. 窗口与容器

### 1.1 弹出面板：窗口属性

`FloatingPanel` 是 `NSPanel` 子类，`FloatingPanel<ContentView>`，全局只有一个实例，在 `applicationDidFinishLaunching` 创建。

| 属性 | 值 | 来源 |
|---|---|---|
| styleMask | `[.nonactivatingPanel, .resizable, .closable, .fullSizeContentView]` | `ManisPocket/FloatingPanel.swift:27` |
| 初始 contentRect | `Defaults[.windowSize]` = 450 × 800（同时被写回 `Defaults[.windowSize]`） | `ManisPocket/FloatingPanel.swift:35`、`ManisPocket/Extensions/Defaults.Keys+Names.swift:61`、`ManisPocket/AppDelegate.swift:105-106` |
| `animationBehavior` | `.none`（无系统开窗动画） | `ManisPocket/FloatingPanel.swift:38` |
| `isFloatingPanel` | true | `ManisPocket/FloatingPanel.swift:39` |
| 窗口层级 | `.statusBar`（高于普通窗口，低于系统弹窗） | `ManisPocket/FloatingPanel.swift:40` |
| `collectionBehavior` | `[.auxiliary, .stationary, .moveToActiveSpace, .fullScreenAuxiliary]` | `ManisPocket/FloatingPanel.swift:41` |
| 标题栏 | 隐藏、透明、无分隔线（`titleVisibility=.hidden`、`titlebarAppearsTransparent=true`、`titlebarSeparatorStyle=.none`） | `ManisPocket/FloatingPanel.swift:42-47` |
| 红绿灯按钮 | 全部隐藏（close/miniaturize/zoom） | `ManisPocket/FloatingPanel.swift:50-52` |
| 背景色 | `.clear`（完全透明，靠内容里的模糊层） | `ManisPocket/FloatingPanel.swift:46` |
| 失活时隐藏 | `hidesOnDeactivate = false`（自己实现关闭逻辑） | `ManisPocket/FloatingPanel.swift:45` |
| `isMovableByWindowBackground` | true（但 popupPosition == .statusItem 时 `isMovable` 恒 false） | `ManisPocket/FloatingPanel.swift:11-14,44` |
| 内容视图圆角 | `contentView.layer.cornerRadius = Popup.cornerRadius + Popup.horizontalPadding` → **9**（`[26]` **12**） | `ManisPocket/FloatingPanel.swift:63` |
| 内容安全区 | `.ignoresSafeArea()`（标题栏会干扰几何，必须忽略） | `ManisPocket/FloatingPanel.swift:57` |
| `canBecomeKey` | true（面板内文本框要能输入） | `ManisPocket/FloatingPanel.swift:210-212` |

**窗口高度动态调整**：
- `open(height:)` 调 `setContentSize(width: min(frame.width, windowSize.width), height: min(height, windowSize.height))`（`ManisPocket/FloatingPanel.swift:76`）。
- 列表内容高度变化后通过 `verticallyResize(to:)` 改高，锚定**底边**（y 补偿 `frame.height - newSize.height`），动画 **0.2s**（`ManisPocket/FloatingPanel.swift:89-99`）。

### 1.2 弹出面板：打开 / 关闭

**打开方式（4 种）**

1. **全局快捷键**（默认 ⇧⌘C）：`KeyboardShortcuts.Name.popup = Shortcut(.c, modifiers: [.command, .shift])`（`ManisPocket/Extensions/KeyboardShortcuts.Name+Shortcuts.swift:4`）。按下时若面板关闭 → `open(height: popup.height)` 并进入 `.opening` 状态，同时**禁用**全局快捷键、改由本地事件监听器接管（`ManisPocket/Observables/Popup.swift:113-119`）。
2. **点击状态栏图标**：`performStatusItemClick` → `panel.toggle(height: popup.height, at: .statusItem)`，即位置强制为「状态栏图标下方」（`ManisPocket/AppDelegate.swift:183`）。
3. **App 被重新激活**（Dock / 再次 open）：`applicationShouldHandleReopen` → `panel.toggle(height: popup.height)`（`ManisPocket/AppDelegate.swift:115-118`）。
4. **`applicationShouldHandleReopen` 之外的菜单栏场景**：`MenuBarExtra` 是空壳且永远隐藏（`ManisPocket/ManisPocketApp.swift:25-29`），仅为了绕开「不能有无 scene 的 App」。

**状态栏图标交互细节（易漏）**
- `⌥ + 点击图标`：切换 `Defaults[.ignoreEvents]`（暂停记录），**不打开面板**（`ManisPocket/AppDelegate.swift:168-181`）。
- `⌥⇧ + 点击图标`：把 `ignoreOnlyNextEvent` 设为当前 `ignoreEvents` 值（只忽略下一次）（`ManisPocket/AppDelegate.swift:175-177`）。
- 当 `ignoreEvents == true` 或 `enabledPasteboardTypes` 为空时，状态栏按钮显示为 disabled 外观（`appearsDisabled`）（`ManisPocket/AppDelegate.swift:20-22,84-93`）。
- 状态栏图标可选 4 种：`manispocket`（SF Symbol `doc.on.clipboard`）、`clipboard`、`scissors`、`paperclip`（`ManisPocket/MenuIcon.swift:5-25`）。
- 勾选「在菜单图标旁显示最近复制」时，按钮标题 = 最近一条未固定项文本，去掉所有换行，`shortened(to: 20)`（`ManisPocket/Observables/AppState.swift:32-37`、`ManisPocket/AppDelegate.swift:73-81`）。

**关闭方式**
- **失焦即关**：`override resignKey()` → 若 `NSApp.alertWindow == nil` 则 `close()`（有系统弹窗时不关，避免确认框把面板吃掉）（`ManisPocket/FloatingPanel.swift:192-199`）。
- `Esc` → `popup.close()`（`ManisPocket/Views/KeyHandlingView.swift:159-161`）。
- 选中条目后自动关闭（`ManisPocket/Observables/History.swift:371-389`）。
- 再次按 ⇧⌘C：`.opening` 状态下松开修饰键 → 转 `.toggle`；`.toggle` 状态且修饰键完全匹配 → 关闭（`ManisPocket/Observables/Popup.swift:146-159,175-178`）。
- `close()` 之后：`preview.state = .closed`、`isPresented = false`、状态栏按钮取消高亮、回调 `onClose` → `popup.reset()`（恢复 `.toggle` 并重新启用全局快捷键）（`ManisPocket/FloatingPanel.swift:201-207`、`ManisPocket/Observables/Popup.swift:79-86`、`ManisPocket/AppDelegate.swift:109`）。

**窗口高度计算**（`Popup`）

```
resize(contentHeight):
    total = contentHeight + headerHeight + extraTopHeight + extraBottomHeight + footerHeight   // Popup.swift:108
    target = preferredHeight(total)                                                          // Popup.swift:109
    verticallyResize(target)                                                                 // 0.2s 动画

preferredHeight(h):                                                                          // Popup.swift:92-105
    minH = 0
    if preview.isOpen && navigator.leadSelection != nil: minH += 150   // minimumPreviewHeight
    minH = max(headerHeight + 5, minH)
    h = max(h, minH)
    h = min(h, Defaults[.windowSize].height)      // 默认上限 800
```

其中 `headerHeight`、`footerHeight`、`extraTopHeight`（paste stack + 顶部固定区）、`extraBottomHeight`（底部固定区）都是**实测高度**回写（`ManisPocket/Views/HeaderView.swift:58`、`ManisPocket/Views/FooterView.swift:60`、`ManisPocket/Views/HistoryListView.swift:97,161`）。

### 1.3 弹出面板：位置规则

枚举 `PopupPosition`：`cursor` / `statusItem` / `window` / `center` / `lastPosition`，默认 `.cursor`（`ManisPocket/PopupPosition.swift:5-10`、`ManisPocket/Extensions/Defaults.Keys+Names.swift:45`）。

`origin(size:statusBarButton:)` 计算左下角坐标（macOS 坐标系原点在左下）：

| 模式 | 规则 | 来源 |
|---|---|---|
| `.center` | 屏幕 `visibleFrame` 居中 | `ManisPocket/PopupPosition.swift:34-37` |
| `.window` | 当前前台 App 窗口 frame 居中 | `ManisPocket/PopupPosition.swift:38-41` |
| `.statusItem` | 状态栏按钮的屏幕矩形：左上角对齐按钮左边缘、顶边贴在按钮底边（`y = screenRect.minY - size.height`）；若右溢出则 `x = screen.maxX - size.width` | `ManisPocket/PopupPosition.swift:42-54` |
| `.lastPosition` | `windowPosition`（归一化 0..1，默认 `(0.5, 0.8)`）→ `anchorX = frame.minX + frame.width*pos.x`，`anchorY = frame.minY + frame.height*pos.y`；窗口以**顶边中点**对齐锚点：`origin = (anchorX - size.width/2, anchorY - size.height)` | `ManisPocket/PopupPosition.swift:55-62`、`ManisPocket/Extensions/Defaults.Keys+Names.swift:62` |
| `.cursor`（默认分支） | `NSEvent.mouseLocation` 后 `y -= size.height`，即鼠标在窗口**左上角** | `ManisPocket/PopupPosition.swift:67-69` |

- 目标屏幕：`Defaults[.popupScreen]`，0 = 主屏，否则 `NSScreen.screens[n-1]`（`ManisPocket/Extensions/NSScreen+ForPopup.swift:5-12`）。
- 用户拖动窗口后回写位置：`anchorX = frame.minX + contentWidth/2 - screen.minX`，`anchorY = frame.maxY - screen.minY`，各自除以屏幕宽/高归一化（`ManisPocket/FloatingPanel.swift:108-117`）。

### 1.4 弹出面板：垂直结构（自上而下）

```
┌─ 窗口（圆角 9，背景 .popover 模糊）────────────────┐
│  HeaderView            padding top 5, h 10        │  ← 高 28（5 + 23）
│  AppFilterBar          高 26（无应用时隐藏）        │
│  ┌ 列表区 padding h 5 ────────────────────────┐   │
│  │  [paste stack]                             │   │
│  │  [顶部固定区]                               │   │
│  │  ── 分隔线（若上方有块）──                   │   │
│  │  ScrollView（上下 padding 6）               │   │
│  │  FooterView                                 │   │
│  └────────────────────────────────────────────┘   │
└───────────────────────────────────────────────────┘
```

结构代码：`ManisPocket/Views/ContentView.swift:18-53`。要点：

- 背景层是 `ZStack` 最底层：macOS 26 用 `GlassEffectView`（`NSGlassEffectView`，style `.regular`），否则 `VisualEffectView`（`NSVisualEffectView`，material `.popover`，blending `.behindWindow`）（`ManisPocket/Views/ContentView.swift:11-16`、`ManisPocket/Views/VisualEffectView.swift:3-31`）。
- `VStack(spacing: 0)`，列表区 `frame(minHeight: 0)` + `layoutPriority(1)`（`ManisPocket/Views/ContentView.swift:19,51-52`）。
- 列表区整体 `.padding(.horizontal, Popup.horizontalPadding)` = 5（`ManisPocket/Views/ContentView.swift:41`）。**Header 与 AppFilterBar 不吃这个 5**，它们各自有内边距（见 §3、§5）。
- 键盘事件处理包在最外层 `KeyHandlingView`（`ManisPocket/Views/ContentView.swift:18`）。
- 列表/贴栈变化时的隐式动画：`.animation(.default.speed(3), value: items)`、`.animation(.default.speed(3), value: pasteStack?.id)`（`ManisPocket/Views/ContentView.swift:36-40`）——即 SwiftUI 默认动画时长除以 3，量级 ≈ 0.12–0.2s，GPUI 用 `ease_out 120ms` 近似。
- 搜索显隐：`.animation(.easeInOut(duration: 0.2), value: searchVisible)`（`ManisPocket/Views/ContentView.swift:60`）。
- 面板内部所有子视图共享 `scenePhase`：监听 `NSWindow.didBecomeKey/didResignKey`，且只认 identifier == bundle id 的那个窗口（`ManisPocket/Views/ContentView.swift:64-80`）。

**典型高度算例**（搜索可见 + 有筛选栏 + 10 行 + 显示 footer，macOS 14 数值）：
`28 (header) + 26 (filter) + 6 + 10×22 + 6 (list) + [1 + 6 + 5×22 + 5] (footer 122) = 408`。

### 1.5 设置窗口

由第三方 `Settings` SPM 包提供（`SettingsWindowController`），样式 `.toolbarItems`（默认参数）：

- 窗口 styleMask `[.titled, .closable]`，contentRect = 第一个 pane 的 view bounds（`build/SourcePackages/checkouts/Settings/Sources/Settings/SettingsWindowController.swift:36-44`）。
- `toolbarStyle = .preference`（macOS 11+），标题可见（`…/SettingsWindowController.swift:50-59`）。
- 布局：**顶部工具栏**（每个 pane 一个 toolbar item，图标 + 文字标签，居中排布），下方是 pane 内容区（`build/.../ToolbarItemStyleViewController.swift:20-49`）。
- 首次显示时 `window.center()`，随后用 autosave name `com.sindresorhus.Settings.FrameAutosaveName` 恢复用户尺寸/位置（`…/SettingsWindowController.swift:109-117`）。
- 打开时会 `NSApp.activate`（`…/SettingsWindowController.swift:95-103`）。

**Pane 顺序与图标**（`ManisPocket/Observables/AppState.swift:117-171`）：

| # | Pane | 标题 key | 中文 | 图标 | 来源 |
|---|---|---|---|---|---|
| 1 | general | `Title`@GeneralSettings = "General" | 通用 | `gearshape` | AppState.swift:118-124 |
| 2 | storage | "Storage" | 存储 | `externaldrive` | AppState.swift:125-131 |
| 3 | sync | "Sync" | （无 zh 表，回退英文） | `arrow.triangle.2.circlepath` | AppState.swift:132-141 |
| 4 | appearance | "Appearance" | 外观 | `paintpalette` | AppState.swift:142-148 |
| 5 | pins | "Pins" | 固定 | `pin.circle` | AppState.swift:149-156 |
| 6 | ignore | "Ignore" | 忽略 | `nosign` | AppState.swift:157-163 |
| 7 | advanced | "Advanced" | 高级 | `gearshape.2` | AppState.swift:164-170 |

> 注意：Settings 的本地化表只有 6 个语言目录下的 6 个文件（Advanced/Appearance/General/Ignore/Pins/Storage），**没有 SyncSettings 的 zh-Hans 表**，Sync pane 内的文案全部是硬编码英文字面量（`ManisPocket/Settings/SyncSettingsPane.swift:27-246`）。

**`Settings.Container` / `Settings.Section` 布局规则（照抄即可，无需 SwiftUI）**

```
Container(contentWidth: 550)
  - 整体 VStack(alignment: .settingsSectionLabel)（所有 section 共用一条标签右对齐基线）
  - .frame(width: 550, alignment: .leading)
  - .padding(.vertical, 20)
  - .padding(.horizontal, 30)
  → pane 总宽 = 550 + 60 = 610（SettingsLayout.paneWidth）

Section:
  - HStack(alignment: .firstTextBaseline) { label(右对齐)  content  Spacer() }
  - label 列宽 = 所有 section 中 label 的最大实测宽度（不是固定值；GPUI 需要两趟测量或预设宽度）
  - content 是 VStack(alignment: .leading)，内部控件垂直堆叠
  - bottomDivider == true 时，在 section 之后插一条 Divider，frame(width: 550, height: 20)
```

（来源：`build/SourcePackages/checkouts/Settings/Sources/Settings/Container.swift:44-67`、`Section.swift:112-119`、`Section.swift:78-79`）

**label 右对齐实现（GPUI 关键）**：Settings 用 `PreferenceKey` 收集最大 label 宽度，所有 section 的第一列统一为该宽度并右对齐（`Section.swift:13-22,112-118`）。GPUI 没有隐式测量副作用，建议**先渲染一遍量出最大宽**（或按最长标签文本宽硬编码一个 120px 常量，再把实际值写死）。

---

## 2. 历史列表项

### 2.1 行的结构（从左到右）

`ListItemView` 是行模板，被历史项、固定项、footer 项、贴栈项共用（`ManisPocket/Views/ListItemView.swift:31-119`）：

```
HStack(spacing: 0)                          // ListItemView.swift:50
 ├─ [可选] 应用图标                          // 仅当 showApplicationIcons
 │    VStack { Spacer; Image(15×15); Spacer }
 │    .padding(.leading, 4).padding(.vertical, 5)      // :51-59
 ├─ Spacer 宽 5（有图标）或 10（无图标）               // :61-62
 ├─ [可选] accessoryImage（颜色片 12×12）              // :64-69
 │    .padding(.trailing, 5).padding(.vertical, 5)
 ├─ [可选] 缩略图 Image                                // :71-75
 │    .padding(.trailing, 5).padding(.vertical, 5)
 │    或 ListItemTitleView（标题文字）                 // :76-79
 │        .padding(.trailing, 5)
 ├─ Spacer                                            // :81
 └─ HStack(spacing: 5)                                 // :83
      ├─ [可选] 多选序号胶囊                           // :84-94
      └─ [可选] 快捷键提示（KeyboardShortcutView）      // :96-105
      .padding(.trailing, 10)                          // :107
  
.frame(minHeight: Popup.itemHeight)                    // :109
.frame(maxWidth: .infinity, alignment: .leading)       // :111
.foregroundStyle(isSelected ? .white : .primary)        // :112
.background(isSelected ? Color.accentColor.opacity(0.8) : .white.opacity(0.001))  // :115
.clipShape(selectionAppearance.rect(cornerRadius: Popup.cornerRadius))            // :116
```

**关键结论**
- 行高：**最小** 22（`[26]` 24），有缩略图时会被撑高（缩略图最高 40 + 上下各 5 → 行可达 50）（`ManisPocket/Views/ListItemView.swift:109`、`ManisPocket/Observables/HistoryItemDecorator.swift:17-19`）。
- 选中底色：`accentColor` + **0.8** 不透明度，非选中为透明（源里的 `.white.opacity(0.001)` 只是 macOS 26 悬停 bug 的 workaround，移植时**不要**真的画一层白）（`ManisPocket/Views/ListItemView.swift:113-115`）。
- 选中文字：**白色**（`ManisPocket/Views/ListItemView.swift:112`）。
- 行圆角：`Popup.cornerRadius`（4 / `[26]` 7），且**四角可分别设置**——多选连续选中时上下相邻行需要"拼接"成一体，见 §2.4。
- 文字：单行，**中间截断** `truncationMode(.middle)`（`ManisPocket/Views/ListItemTitleView.swift:11-12,16-17`）。GPUI 无中间截断，需自实现（见 §10）。

### 2.2 标题内容与图片

- 标题文本来源：`item.title`；搜索时用 `item.attributedTitle`（带高亮的 AttributedString）（`ManisPocket/Views/HistoryItemView.swift:55,61`、`ManisPocket/Observables/HistoryItemDecorator.swift:180-229`）。
- **图片项**：`image`（缩略图）非空时**替代标题文字显示**，不再画文本（`ManisPocket/Views/ListItemView.swift:71-79`）。缩略图尺寸 `NSSize(width: 340, height: Defaults[.imageMaxHeight]=40)`，等比缩放且**不放大**（`ManisPocket/Observables/HistoryItemDecorator.swift:17-19`、`ManisPocket/Extensions/NSImage+Resized.swift:12-33`）。
- **颜色片**：文本项若标题恰为十六进制颜色字符串（如 `#FF0000`），在标题前显示 12×12 色块（`ManisPocket/Views/HistoryItemView.swift:54`、`ManisPocket/ColorImage.swift:5-16`）。
- 标题超长截断：显示用 `title` 已先被 `shortened(to: 100)`（`ManisPocket/Observables/HistoryItemDecorator.swift:63` 为 10000；`AppState.menuIconText` 才是 100/20）。实际高亮时再 `shortened(to: 500)`（`:186`）。

### 2.3 行内交互（hover 才出现）

`HistoryItemView.normalView` 是一个 `ZStack(alignment: .trailing)`，hover 时右侧出现按钮组、行本身右缩 56：

| 元素 | 规格 | 来源 |
|---|---|---|
| hover 时行右内距 | `56`，动画 `easeInOut 0.15s` | `ManisPocket/Views/HistoryItemView.swift:63-64` |
| 按钮组间距 | `HStack(spacing: 2)`，`.padding(.trailing, 8)`，`.transition(.opacity)` | `ManisPocket/Views/HistoryItemView.swift:68,103-104` |
| 按钮顺序 | ①（仅当文件项且来自远端）`icloud.and.arrow.down`「Download file」② `doc.on.doc`「复制到剪贴板」③ `trash`「删除」 | `ManisPocket/Views/HistoryItemView.swift:69-101` |
| 按钮尺寸 | `frame(width: 24, height: 22)`，圆角 `5` | `ManisPocket/Views/ActionIconButton.swift:14,17` |
| 按钮图标 | `.system(size: 11, weight: .semibold)` | `ManisPocket/Views/ActionIconButton.swift:13` |
| 按钮默认色 | 选中行 → `white.opacity(0.6)`；未选中 → `.secondary` | `ManisPocket/Views/ActionIconButton.swift:52` |
| hover 背景 | trash → `red.opacity(isDark ? 0.5 : 0.15)`；其余 → `blue.opacity(isDark ? 0.5 : 0.15)` | `ManisPocket/Views/ActionIconButton.swift:37-42` |
| hover 前景 | trash → `white`（选中行）或 `red`；其余 → `white`（选中行）或 `blue` | `ManisPocket/Views/ActionIconButton.swift:46-51` |
| hover 动画 | `easeInOut 0.1s` | `ManisPocket/Views/ActionIconButton.swift:22-24` |

**复制成功气泡（Copied!）**
- 文案 `copied_toast` = "Copied!" / "已复制"。
- 样式：`.caption` 字号，`padding h6 / v2`，底 `RoundedRectangle(cornerRadius: 4).fill(Color.green.opacity(0.85))`，前景**白色**，`.padding(.trailing, 8)`，`transition(.opacity)`。
- 停留 **1.5 秒**后消失（`Task.sleep(for: .seconds(1.5))`）。
- 来源：`ManisPocket/Views/HistoryItemView.swift:88-92,108-120`。

### 2.4 选中外观（多选拼接）

`SelectionAppearance` 决定四角圆角（`ManisPocket/Views/ListItemView.swift:4-29`）：

| 情况 | 圆角 |
|---|---|
| `.none`（默认） | 四角都是 `cornerRadius` |
| `.topConnection`（上一行被选中、下一行没有） | 只有下方两角圆 |
| `.bottomConnection`（下一行被选中、上一行没有） | 只有上方两角圆 |
| `.topBottomConnection`（上下都被选中） | 四角全 0 |

判定：看 `previous?.isSelected` / `next?.isSelected` 的组合（`ManisPocket/Views/HistoryItemView.swift:22-35`）。

**多选序号徽标**（多选进行中才显示）：
- `Text("\(index+1)")`，`.caption`，`frame(minWidth: 10, alignment: .center)`，`padding(3)`，
- 背景 `Color.secondary.opacity(isSelected ? 0.5 : 0.8)`，`Capsule()`，
- 前景**白色**。
- 来源：`ManisPocket/Views/ListItemView.swift:84-94`。
- 触发条件：`appState.navigator.isMultiSelectInProgress && item.selectionIndex >= 0`（`ManisPocket/Views/HistoryItemView.swift:15-20`）。
- 注意：`AppState.multiSelectionEnabled` 当前**硬编码 false**（`ManisPocket/Observables/AppState.swift:12`），所以多选/贴栈在正式构建里是关闭的 —— 但代码路径完整，移植时应保留为可开关特性。

### 2.5 快捷键提示（行右侧）

`KeyboardShortcutView`（`ManisPocket/Views/KeyboardShortcutView.swift:3-28`）：
- 内容 = 修饰符符号串（按 `⌃⌥⇧⌘` 顺序拼接，见 §8.3）+ 最后一个字符。
- 布局：`HStack(spacing: 1)`；每个修饰符一个 `Text`；字符 `Text(character).frame(width: 12, alignment: .center)`。
- `lineLimit(1)`；字符为空时整体 `opacity 0`；否则 `opacity 0.7`。
- 字体：默认正文字体（13pt）。
- 显隐由 `KeyShortcut.isVisible(all:pressed:)` 决定（见 §8.4），不可见时 `frame(width: 0)`（`ManisPocket/Views/ListItemView.swift:96-105`）。

**行上提示的快捷键内容**：
- 未固定项：前 9 个可见项分别显示 `1`…`9` 三套组合（`ManisPocket/Observables/History.swift:633-644`）。
- 固定项：显示其 pin 字符（单个字母）三套组合（`ManisPocket/Observables/History.swift:623-631`）。
- 三套组合由 `KeyShortcut.create(character:)` 生成（`ManisPocket/KeyShortcut.swift:6-13`）：
  1. `[.command]` + 字符
  2. `[.option]` + 字符
  3. `[Defaults[.pasteByDefault] ? .command : .option, .shift]` + 字符

### 2.6 编辑态

双击/右键 Edit 进入 `editingView`（`ManisPocket/Views/HistoryItemView.swift:185-224`）：

```
HStack(spacing: 0)
 Spacer().frame(width: 10)
 TextField（多行，lineLimit 1...5，plain 样式，自动聚焦）
 Spacer().frame(width: 4)
 Button  ✔ checkmark.circle.fill  foreground .green  .padding(.trailing, 4)
 Button  ✕ xmark.circle.fill      foreground .secondary .padding(.trailing, 8)
.frame(minHeight: Popup.itemHeight)
.frame(maxWidth: .infinity, alignment: .leading)
.background(Color.accentColor.opacity(0.15))
.clipShape(.rect(cornerRadius: Popup.cornerRadius))
```
- 回车保存、Esc 取消（`ManisPocket/Views/HistoryItemView.swift:195-200`）。
- 保存逻辑：空文本 → 删除该项；否则更新文本（`ManisPocket/Observables/History.swift:406-422`）。

### 2.7 右键菜单

仅在 `!item.title.isEmpty` 时提供「Edit」；「Pin/Unpin」总是提供；远端文件项额外提供「Download <fileName>」；「Delete」（`ManisPocket/Views/HistoryItemView.swift:149-182`）。删除走确认对话框：

- 标题 `delete_alert_message` = "Are you sure you want to delete this item?" / 中文「确认删除？」
- 说明 `delete_alert_comment` = "This action cannot be undone." / 「无法撤消此操作。」
- 按钮 `delete_alert_confirm` = "Delete" / 「删除」，`delete_alert_cancel` = "Cancel" / 「取消」
- 来源：`ManisPocket/Views/HistoryItemView.swift:137-148`、`ManisPocket/en.lproj/Localizable.strings:34-37`

### 2.8 列表容器与分隔线

`HistoryListView`（`ManisPocket/Views/HistoryListView.swift:69-162`）：

- 列表项容器 `LazyVStack(spacing: 0)`（`ManisPocket/Views/MultipleSelectionListView.swift:10`）→ **行间无间距**。
- 滚动区上下 padding：
  - `top = Popup.verticalSeparatorPadding` = 6（`:41-43`）
  - `bottom = showFooter ? 6 : 5`（`:45-49`）
- 分隔线（`Divider`）三处，均 `padding(.horizontal, 6)`：
  - `topSeparator()`：额外 `padding(.top, 6)`（`:51-55`）
  - `bottomSeparator()`：额外 `padding(.bottom, 6)`（`:57-61`）
  - `separator()`（贴栈与顶部固定区之间）：额外 `padding(.vertical, 6)`（`:63-67`）
- **顶部区可见条件**：`topPinsVisible || pasteStackVisible`；**底部区**：`pinTo == .bottom && pinsVisible`（`:70-75`）。
- 滚动指示器内容边距：leading 10、top/bottom = 对应 padding（`:146-148`）。
- 滚动到选中项：`task(id: scrollTarget)` 延迟 **10ms** 再 `proxy.scrollTo(selection)`（`:106-116`）。
- 窗口激活时的行为（关键交互）：
  - `scenePhase == .active` → 聚焦搜索框、`isKeyboardNavigating = true`、选中首个未固定项（没有则首个固定项）、`preview.enableAutoOpen(); resetAutoOpenSuppression(); startAutoOpen()`
  - `scenePhase == .background` → 清空修饰键、`isKeyboardNavigating = true`、`preview.cancelAutoOpen()`
  - 来源：`ManisPocket/Views/HistoryListView.swift:117-130`

---

## 3. 顶部搜索区

### 3.1 Header 整体

`HeaderView`（`ManisPocket/Views/HeaderView.swift:14-59`）：

```
HStack(alignment: .top, spacing: 0)
 ├─ HStack(alignment: .center, spacing: 0)
 │   ├─ ListHeaderView(...).padding(.horizontal, 5)        // :17-21
 │   └─ ToolbarButton(预览开关)
 │        Image(systemName: previewPlacement == .right ? "sidebar.left" : "sidebar.right")
 │        .padding(.trailing, 5)                            // :23-37
 │   .opacity(searchVisible ? 1 : 0)                        // :39
 │   .layoutPriority(1)
 └─ [searchVisible 时] Button { history.addNew() } Image("plus").foregroundStyle(.secondary)
      .help("add_new")                                      // :42-51
.padding(.top, 5)          // Popup.verticalPadding          // :53
.padding(.horizontal, 10)                                   // :54
.animation(.default.speed(3), value: leadSelection)          // :55
.frame(maxHeight: searchVisible ? nil : 0, alignment: .top)  // :57
```

- 预览按钮高度 **23**（`ManisPocket/Views/ToolbarView.swift:41`）。
- 搜索不可见时整个 header 高度收为 0，搜索框用 `offset(y: -Popup.itemHeight)` 移出视野（`ManisPocket/Views/HeaderView.swift:57`、`ManisPocket/Views/ListHeaderView.swift:31`）。
- `searchVisible = Defaults[.showSearch] && (searchVisibility == .always || !searchQuery.isEmpty)`（`ManisPocket/Observables/AppState.swift:22-30`）。默认 `showSearch = true`、`searchVisibility = .always`（`ManisPocket/Extensions/Defaults.Keys+Names.swift:53-54`）。

### 3.2 应用标题

- 文案**硬编码** "Manis Pocket"（不本地化）。
- `.foregroundStyle(.secondary)`，`.padding(.leading, 5)`，默认正文字体。
- 仅 `Defaults[.showTitle]` 为 true 时显示（默认 true）。
- 来源：`ManisPocket/Views/ListHeaderView.swift:16-20`、`ManisPocket/Extensions/Defaults.Keys+Names.swift:56`。

### 3.3 搜索输入框

`SearchFieldView`（`ManisPocket/Views/SearchFieldView.swift:9-43`）：

```
ZStack
  ├─ RoundedRectangle(cornerRadius: Popup.cornerRadius, style: .continuous)
  │    .fill(Color.secondary).opacity(0.1)
  │    .frame(height: 23)                          // :11-14
  └─ HStack
       ├─ Image(systemName: "magnifyingglass")
       │    .frame(width: 11, height: 11)
       │    .padding(.leading, 5)
       │    .opacity(0.8)                          // :17-20
       ├─ TextField(placeholder)
       │    .disableAutocorrection(true)
       │    .lineLimit(1)
       │    .textFieldStyle(.plain)
       │    onSubmit { appState.select() }         // :22-28
       └─ [query 非空时] Button { query = "" }
            Image(systemName: "xmark.circle.fill")
            .frame(width: 11, height: 11)
            .padding(.trailing, 5)
            .opacity(0.9)                          // :30-40
```

**关键数值**：框高 **23**、圆角 **4**（`[26]` 7）、底色 = secondary @ **10%**、放大镜 11×11 / 左侧 5 / 不透明度 0.8、清除按钮 11×11 / 右侧 5 / 0.9。

- placeholder：`search_placeholder` = "type to search…" / 「搜索…」（`ManisPocket/en.lproj/Localizable.strings:1`、`ManisPocket/zh-Hans.lproj/Localizable.strings:1`）。
- 回车 = 选中当前项（效果同 Return 键，见 §7）。
- 失焦时（`scenePhase` 变 background）若 query 非空 → 清空（`ManisPocket/Views/ListHeaderView.swift:25-29`）。
- 打开面板后自动聚焦（`ManisPocket/Views/ContentView.swift:42-44`）。

### 3.4 搜索算法与高亮

**4 种模式**（`ManisPocket/Search.swift:6-28`、默认 `.exact`）：

| 模式 | 实现 | 说明 |
|---|---|---|
| `exact`（默认） | `range(of:options: .caseInsensitive)` | 子串匹配，返回首个 range |
| `regexp` | `range(of:options: .regularExpression)` | 正则 |
| `fuzzy` | `Fuse(threshold: 0.7)`，结果按 score 升序；单条文本超过 5000 字符截断 | 模糊 |
| `mixed` | 依次尝试 exact → regexp → fuzzy，任一有结果即返回 | 混合 |

（`ManisPocket/Search.swift:38-132`）

**触发与节流**：`searchQuery` 的 `didSet` 经 `Throttler(minimumDelay: 0.2)` 节流（**200ms**），然后 `core.search(...)` → 更新列表 → 若 query 为空则选中首个未固定项，否则 `highlightFirst()` → `popup.needsResize = true`（`ManisPocket/Observables/History.swift:25-41,105`）。

**匹配高亮**（`HistoryItemDecorator.highlight`，`ManisPocket/Observables/HistoryItemDecorator.swift:180-229`）：

| `Defaults[.highlightMatch]` | 样式 | 备注 |
|---|---|---|
| `.bold`（默认） | `font = .bold(.body)()` | 加粗，字号不变 |
| `.italic` | `font = .italic(.body)()` | 斜体 |
| `.underline` | `underlineStyle = .single` | 下划线 |
| `.color` | `backgroundColor = .findHighlightColor`，`foregroundColor = .black` | 系统查找高亮底色 + 黑字 |

- 高亮的范围来自搜索结果的 `ranges`（Unicode scalar 偏移），需要做 scalar → grapheme 的映射（源码里 `scalarBoundaries` 那段，`:190-211`）。GPUI 侧用 `TextRun`，文本本身按 UTF-8 字节切片，需自己做 char→byte 索引。
- 标题在建立 attributed string 前先 `shortened(to: 500)`（`:186`）。

### 3.5 筛选状态与搜索的关系

- `selectedApp`、`excludedDevices` 变化也会重新跑搜索并 `needsResize`（`ManisPocket/Observables/History.swift:43-67`）。
- 选中条目复制后会把 `searchQuery` 清空（`ManisPocket/Observables/History.swift:394-396`）。

---

## 4. 底部 Footer

### 4.1 结构

`FooterView`（`ManisPocket/Views/FooterView.swift:21-61`）：

```
VStack(spacing: 0)
 ├─ Divider()
 │    .padding(.horizontal, 6)      // horizontalSeparatorPadding
 │    .padding(.bottom, 6)          // verticalSeparatorPadding
 ├─ ZStack   // 第 1 项与第 2 项叠在一起，靠 opacity 交叉淡入
 │    ├─ FooterItemView(items[0]).opacity(clearOpacity)       // 默认 1
 │    └─ FooterItemView(items[1]).opacity(clearAllOpacity)    // 默认 0
 ├─ ForEach(items[2...]) { FooterItemView($0) }
.opacity(showFooter ? 1 : 0)
.frame(maxHeight: showFooter ? nil : 0)
.padding(.bottom, showFooter ? Popup.verticalPadding : 0)   // 5
```

- Footer 项本身复用 `ListItemView`，所以**每项高 22（`[26]` 24）**，文字默认 13pt（`ManisPocket/Views/FooterItemView.swift:8-11`）。
- 显示开关 `Defaults[.showFooter]`（默认 **true**）（`ManisPocket/Extensions/Defaults.Keys+Names.swift:50`）。
- 高度回写 `popup.footerHeight`（`ManisPocket/Views/FooterView.swift:60`）。
- **鼠标 hover 到任何 footer 项时，如果预览开着会自动关闭预览**（`ManisPocket/Views/FooterItemView.swift:13-17`）。

### 4.2 5 个 footer 项（完整清单）

| # | title key | 中文 | 快捷键 | tooltip key / 文案 | 动作 | 来源 |
|---|---|---|---|---|---|---|
| 1 | `clear` | 清除 | `⌥⌘⌫`（KeyShortcut(.delete, [.command,.option])） | `clear_tooltip` = "Clear unpinned items.\nSelect with ⇧ to clear all." /「清除未固定的项目。\n按住 ⇧ 以全部清除。」 | `history.clear()`（只清未固定） | `ManisPocket/Observables/Footer.swift:30-45` |
| 2 | `clear_all` | 全部清除 | `⇧⌥⌘⌫`（[.command,.option,.shift]） | `clear_all_tooltip` = "Clear all items." /「清除全部项目。」 | `history.clearAll()` | `ManisPocket/Observables/Footer.swift:46-61` |
| 3 | `preferences` | 偏好设置… | `⌘,`（KeyShortcut(key: .comma)，默认修饰符 = command） | 无 | `openPreferences()` | `ManisPocket/Observables/Footer.swift:62-69` |
| 4 | `about` | 关于 | **无** | `about_tooltip` = "Read more about the application." /「查看更多应用信息。」 | 打开系统 About 面板 | `ManisPocket/Observables/Footer.swift:70-75` |
| 5 | `quit` | 退出 | `⌘Q`（KeyShortcut(key: .q)） | `quit_tooltip` = "Quit application." /「退出应用。」 | `NSApp.terminate` | `ManisPocket/Observables/Footer.swift:76-82` |

> 快捷键符号的显示格式由 `KeyShortcut.description` 决定：修饰符串 + 字符大写（`ManisPocket/KeyShortcut.swift:20-29`）。所以显示为 `⌥⌘⌫`、`⇧⌥⌘⌫`、`⌘,`、`⌘Q`。

### 4.3 clear → clear_all 的动态切换（易漏交互）

`FooterView.clearAllModifiersPressed`（`ManisPocket/Views/FooterView.swift:13-19`）：

```
按下修饰键集合非空
且 不是 clear 修饰键([⌘,⌥]) 的子集
且 是 clear_all 修饰键([⌘,⌥,⇧]) 的子集
```

成立时：`clearOpacity = 0`、`clearAllOpacity = 1`，并切换 `isVisible`；若当前 footer 选中项是第 1 项，则把选择转移到第 2 项（`ManisPocket/Views/FooterView.swift:33-51`）。松开 ⇧ 恢复。**即：按住 ⇧ 时 footer 第一行的文字从「清除」变成「全部清除」。**

### 4.4 确认对话框

`clear` / `clear_all` 都带确认（`ManisPocket/Observables/Footer.swift:34-40,50-56`）：

- 标题 `clear_alert_message` = "Are you sure you want to clear the history?" / 「确定要清除历史记录吗？」
- 说明 `clear_alert_comment` = "You can't undo this action." / 「无法撤消此操作。」
- 确认 `clear_alert_confirm` = "Clear" / 「清除」；取消 `clear_alert_cancel` = "Cancel" / 「取消」
- 提供"不再询问"抑制开关，绑定 `Defaults[.suppressClearAlert]`（默认 false）（`ManisPocket/Views/ConfirmationView.swift:8-24`、`ManisPocket/Observables/Footer.swift:15-18`、`ManisPocket/Extensions/Defaults.Keys+Names.swift:60`）。
- 通过键盘触发（⌥⌘⌫）时也走同一确认逻辑（`ManisPocket/Views/KeyHandlingView.swift:31-44`）。

### 4.5 footer 的键盘导航

Footer 项参与统一的选择模型：`↑/↓` 可以从列表末项进入 footer 首项，从 footer 首项继续回绕（仅 `allowCycle` 时）（`ManisPocket/Observables/NavigationManager.swift:233-259`）。选中 footer 项时 `leadHistoryItem = nil`（预览区清空）（`ManisPocket/Observables/NavigationManager.swift:179-185`）。

---

## 5. 应用筛选栏（AppFilterBar）

`ManisPocket/Views/AppFilterBar.swift`。**完全为空时整条不渲染**（高度 0）。

### 5.1 容器

```
if !apps.isEmpty || !pairedDevices.isEmpty:
    ScrollView(.horizontal, showsIndicators: false)
      HStack(spacing: 6)                       // :19
        [应用图标...]
        [分隔竖线，仅当两端都有内容]
        [设备图标...]
      .padding(.horizontal, 4)                 // :51
    .frame(height: 26)                         // :53
```

- 图标间距 **6**，左右内边距 **4**，整条高度 **26**。
- 中间分隔线：`Rectangle().fill(Color.primary.opacity(0.15)).frame(width: 1, height: 16)`（`:32-36`）。
- 数据源：`history.sourceApps`（去重后的应用 bundleId + 图标，未知来源用 `bundleId == ""` 表示），以及 `appState.syncDevices`（已配对设备）（`:7-13`、`ManisPocket/Observables/History.swift:69-86`）。

### 5.2 应用图标（AppFilterIcon）

| 属性 | 值 | 来源 |
|---|---|---|
| 尺寸 | `frame(width: 18, height: 18)` | `AppFilterBar.swift:77` |
| 圆角 | `4` | `AppFilterBar.swift:89,91` |
| 变暗（其它应用被选中时） | `opacity(0.45)` | `AppFilterBar.swift:79` |
| 选中底色 | 覆盖 `Color.accentColor.opacity(0.2)` | `AppFilterBar.swift:83` |
| hover 底色 | 覆盖 `Color.primary.opacity(0.08)` | `AppFilterBar.swift:86` |
| 选中描边 | `stroke(accentColor.opacity(0.8), lineWidth: 1)`，圆角 4 | `AppFilterBar.swift:90-93` |
| 未选中/未 hover | 覆盖层为空（透明） | `AppFilterBar.swift:81-87` |
| 未知来源图标 | `questionmark.app.dashed` → 退化 `questionmark` → 应用自身图标（**禁止 force unwrap**） | `AppFilterBar.swift:72-75` |
| tooltip | 未知源显示 "Unknown Source"，否则 bundleId | `AppFilterBar.swift:100` |
| 点击 | `toggleAppSelection(bundleId)`（再点一次取消） | `AppFilterBar.swift:97-99`、`ManisPocket/Observables/History.swift:54-56` |

**三种视觉状态的取值优先级**：变暗（opacity 0.45）与选中/hover 底色**同时生效**（opacity 先作用，再叠 overlay）。

### 5.3 设备图标（DeviceFilterIcon）

被排除的设备 = "变暗 + 红框"：

| 状态 | 背景 | 描边 | 整体不透明度 | 来源 |
|---|---|---|---|---|
| 正常 | `Color.clear` | 无 | 1.0 | `DeviceFilterIcon.swift:16-23` |
| hover | `Color.primary.opacity(0.08)` | 无 | 1.0 | `DeviceFilterIcon.swift:17` |
| **被排除** | `Color.red.opacity(0.1)` | `red.opacity(0.5)`，lineWidth 1，圆角 4 | **0.3** | `DeviceFilterIcon.swift:16,20-23` |

- 内容：`Text(device.icon)`（emoji 字符串，如 💻），`.font(.system(size: 14))`，`frame(width: 18, height: 18)`，`RoundedRectangle(cornerRadius: 4)`（`:11-15`）。
- tooltip：`device.nickname`（`:26`）。
- 点击 = 切换排除集合 `history.excludedDevices`（`ManisPocket/Views/AppFilterBar.swift:43-48`）。

### 5.4 筛选逻辑

`filteredItems()`：`selectedApp == nil && excludedDevices.isEmpty` 时直接返回全部；否则逐项过滤——`selectedApp` 必须等于 `item.application`（nil 视为 `""`）；`item.syncSource` 在 `excludedDevices` 中则排除（`ManisPocket/Observables/History.swift:594-611`）。筛选变化后重跑搜索（`:43-67`）。

---

## 6. 设置页（7 个 pane）

公共布局见 §1.5。以下逐 pane 列出**每一个控件**：类型 / 标签（中文 + 英文 key）/ 默认值 / 作用 / 来源行号。

> 约定：`Toggle` = 系统开关；`Picker` = 下拉；`TextField` = 输入框；`Stepper` = 步进器；`Button` = 按钮；`List` = 表格/列表；`Recorder` = 快捷键录制控件（KeyboardShortcuts 提供，显示当前组合并可点击重录）。

### 6.1 General（通用）

`ManisPocket/Settings/GeneralSettingsPane.swift`。共 9 个 Section。

**Section 1（无标签，bottomDivider = true）** — `:23-38`

| # | 控件 | 标签（英 / 中） | 默认 | 作用 | 行号 |
|---|---|---|---|---|---|
| 1 | Toggle | LaunchAtLogin / 登录时启动 | 系统登录项状态 | 开机自启（LaunchAtLogin 包） | `:24-26` |
| 2 | Toggle | CheckForUpdates / 自动检查更新 | Sparkle 当前值 | 自动检查更新 | `:27-29` |
| 3 | Button | CheckNow / 检查更新 | — | 立即检查更新 | `:30-33` |
| 4 | Toggle | BetaUpdates / 包含测试版更新（help: BetaUpdatesTooltip） | `false` | 更新时也找 beta | `:34-37`、`ManisPocket/Extensions/Defaults.Keys+Names.swift:65` |

**Section 2「Open:」/「打开：」** — `:40-51`
| 控件 | 说明 |
|---|---|
| `KeyboardShortcuts.Recorder(for: .popup)` | 全局打开快捷键，默认 **⇧⌘C**。help = `OpenTooltip`。录制回调里：新值为 nil → 移除事件监听；非 nil → 确保监听已初始化（`:41-50`） |

**Section 3「Pin:」/「固定：」** — `:53-56`：`Recorder(for: .pin)`，默认 **⌥P**，help = `PinTooltip`。
**Section 4「Delete:」/「删除：」** — `:57-60`：`Recorder(for: .delete)`，默认 **⌥⌫**，help = `DeleteTooltip`。
**Section 5「Preview:」/「预览：」（bottomDivider）** — `:61-67`：`Recorder(for: .togglePreview)`，默认 **⌃Space**，help = `ShowPreviewTooltip`。

> 注意：`pin` 与 `delete` 这两个全局快捷键在启动后会被**主动禁用**（只在面板内由 KeyChord 处理），是为了避免全局冲突：`KeyboardShortcuts.disable([.delete, .pin])` 且监听变化后再次 disable（`ManisPocket/AppDelegate.swift:199-212`）。Recorder 仍然可编辑。

**Section 6「Search:」/「搜索：」（bottomDivider）** — `:69-80`
| 控件 | 规格 |
|---|---|
| Picker（`.labelsHidden()`，`frame(width: 180, alignment: .leading)`） | 4 个选项：Exact/精确、Fuzzy/模糊、Regex/正则表达式、Mixed/混合。绑定 `Defaults[.searchMode]`，默认 **`.exact`**（`:73-79`、`ManisPocket/Search.swift:6-28`、`Defaults.Keys+Names.swift:49`） |

**Section 7「Behavior:」/「行为:」（bottomDivider）** — `:82-105`
| # | 控件 | 标签（英 / 中） | 默认 | 作用 |
|---|---|---|---|---|
| 1 | Toggle（`.fixedSize()`） | PasteAutomatically / 自动粘贴 | `false` | 选中后自动模拟粘贴 |
| 2 | Toggle（`.fixedSize()`） | PasteWithoutFormatting / 纯文本粘贴 | `false` | 选中后去格式 |
| 3 | Text（多行，gray，`.controlSize(.small)`，`fixedSize(horizontal:false, vertical:true)`） | `Modifiers` = "Customize the behavior when selecting item:\n• Select with %@ pressed to copy item.\n• Select with %@ pressed to copy and paste item.\n• Select with %@ pressed to copy, clear formatting, and paste item." | 动态 | 说明三个修饰键组合；`%@` 依次填 copy / paste / pasteWithoutFormatting 的修饰符描述（`:98-104`、`GeneralSettingsPane.swift:14-16,135-139`） |

**Section 8「Append Mode:」/（无中文 key）（bottomDivider）** — `:107-123`
| # | 控件 | 标签（英） | 默认 | 作用 |
|---|---|---|---|---|
| 1 | Toggle（help: EnableAppendModeTooltip） | "Enable append mode (CMD+C twice)" | `false`（`appendModeEnabled`） | 连按两次 CMD+C 时把新内容追加到最近一条，而不是新建 |
| 2 | HStack：Text("Time window:") + TextField（`frame(width: 60)`，`.number` 格式）+ Text("seconds") | — | `1.0` 秒（`appendModeTimeWindow`） | 追加的时间窗口；**`appendModeEnabled == false` 时整行 disabled**（`:122`） |

**Section 9（无标签，无分隔线）** — `:125-131`：`Link` 文案 `NotificationsAndSounds` = "Notifications and sounds 􀱁"，指向 `x-apple.systempreferences:com.apple.preference.notifications?id=<bundleId>`（`:8-10,126-130`）。

### 6.2 Storage（存储）

`ManisPocket/Settings/StorageSettingsPane.swift`。3 个 Section。

**Section 1「Save:」/「保存：」（bottomDivider）** — `:75-94`
| # | 控件 | 标签（英 / 中） | 默认 | 作用 |
|---|---|---|---|---|
| 1 | Toggle | Files / 文件 | 由 `enabledPasteboardTypes` 推导 | 保存文件类（`fileURL`）。打开 → 并入 `StorageType.files.types`；关闭 → 移除 |
| 2 | Toggle | Images / 图像 | 同上 | 保存 `heic/jpeg/png/tiff` |
| 3 | Toggle | Text / 文本 | 同上 | 保存 `html/rtf/string` |
| 4 | Text（small、gray） | `SaveDescription` = "Change what types of copied content should be stored." /「设置何种类型的内容会被保存。」 | — | 说明 |

来源：`:79-93`、`ManisPocket/Extensions/Defaults.Keys+Names.swift:4-11,19-21`

**Section 2「Size:」/「数量：」** — `:96-118`
| # | 控件 | 标签 | 默认 | 作用 |
|---|---|---|---|---|
| 1 | Toggle | UnlimitedHistory / 无限历史记录 | `false`（`isUnlimitedHistory`） | 不限条数 |
| 2 | TextField（`frame(width: 80)`，Formatter 1...9999，help: SizeTooltip） | — | `500`（`size`，存储 key 名为 `historySize`） | 保留条数 |
| 3 | Stepper（`1...9999`，labelsHidden） | — | 同上 | 同上 |
| 4 | Text（small、gray、help: CurrentSizeTooltip） | 当前磁盘占用（`ByteCountFormatter`，`.file` 风格） | 动态 | 显示 DB 大小 |

- `isUnlimitedHistory == true` 时：TextField 与 Stepper `disabled`，整行 `opacity(0.5)`（`:105,108,117`）。
- 默认值来源：`Defaults.Keys+Names.swift:57-58`；SizeTooltip 文案里写的是 "Default: 200."，**与代码默认值 500 不一致**，以代码为准。

**Section 3「SortBy:」/「排序方式：」** — `:120-129`
| 控件 | 规格 |
|---|---|
| Picker（labelsHidden，`frame(width: 160, alignment: .leading)`，help: SortByTooltip） | 3 选项：LastCopiedAt/上次复制时间（默认）、FirstCopiedAt/首次复制时间、NumberOfCopies/复制次数（`ManisPocket/Sorter.swift:7-26`、`Defaults.Keys+Names.swift:59`） |

### 6.3 Appearance（外观）

`ManisPocket/Settings/AppearanceSettingsPane.swift`。6 个 Section。

**Section 1「PopupAt:」/「弹窗位置：」** — `:53-80`
| 控件 | 规格 |
|---|---|
| Picker（labelsHidden，`frame(width: 141, alignment: .leading)`，help: PopupAtTooltip） | 5 选项：Cursor/光标（默认）、Menu icon/菜单栏图标、Window center/窗口中心、Screen center/屏幕中央、Last position/最后位置（`ManisPocket/PopupPosition.swift:5-29`） |
| 多屏时 `.center` / `.lastPosition` 两个选项改为**子 Picker**（显示 "Active screen"/活动屏幕 + 各屏幕名），选中即同时设置 `popupScreen` 与 `popupAt`（`:188-221`） |
| 仅 `popupAt == .lastPosition` 时出现的按钮：`Image(systemName: "arrow.uturn.backward.circle.fill").imageScale(.large)`，`.borderless`，help = PopupAtLastLocationReset，点击重置 `windowPosition` 为默认；当已是默认值时 `disabled`（`:68-78`） |

**Section 2「PinTo:」/「固定项目位置：」** — `:82-91`
| 控件 | 规格 |
|---|---|
| Picker（labelsHidden，`frame(width: 141)`，help: PinToTooltip） | Top/顶部（默认）、Bottom/底部（`ManisPocket/PinsPosition.swift:5-19`、`Defaults.Keys+Names.swift:44`） |

**Section 3「ImageHeight:」/「图片高度：」** — `:93-101`
| 控件 | 规格 |
|---|---|
| TextField（`frame(width: 120)`，Formatter 1...200，help: ImageHeightTooltip） | 绑定 `Defaults[.imageMaxHeight]`，默认 **40** |
| Stepper（`1...200`，labelsHidden） | 同上 |

**Section 4「PreviewDelay:」/「预览延迟：」** — `:103-111`
| 控件 | 规格 |
|---|---|
| TextField（`frame(width: 120)`，Formatter 200...100000） | 绑定 `Defaults[.previewDelay]`，默认 **500**（毫秒） |
| Stepper（`200...100_000`） | 同上 |

> 文档不一致：tooltip 文案写 "Default: 1500."（`ManisPocket/Settings/en.lproj/AppearanceSettings.strings:20`），代码默认值为 **500**（`ManisPocket/Extensions/Defaults.Keys+Names.swift:47`）。移植按 **500**。

**Section 5「HighlightMatches:」/「高亮匹配：」（bottomDivider）** — `:113-125`
| 控件 | 规格 |
|---|---|
| Picker（labelsHidden，`frame(width: 141)`，help: HighlightMatchesTooltip） | 4 选项：Color/颜色、Bold/粗体（默认）、Italic/斜体、Underline/强调（=下划线）（`ManisPocket/HighlightMatch.swift:4-26`、`Defaults.Keys+Names.swift:22`） |

**Section 6（无标签，无分隔线）** — `:127-181`，8 项：

| # | 控件 | 标签（英 / 中） | 默认 | 备注 | 行号 |
|---|---|---|---|---|---|
| 1 | Toggle | ShowSpecialSymbols / 显示特殊符号（help: ShowSpecialSymbolsTooltip） | `true` | 决定标题里是否把空格/换行/制表符渲染成 `·`/`⏎`/`⇥` | `:128-131` |
| 2 | HStack：Toggle + Picker | ShowMenuIcon / 显示菜单栏图标 + 图标选择器（4 个图标缩略图，`.scaledToFit()`，`.controlSize(.small)`） | `true` / `manispocket` | Picker 在 `showInStatusBar == false` 时 disabled | `:133-147` |
| 3 | Toggle | ShowRecentCopyInMenuBar / 在菜单栏图标旁显示剪贴板内容 | `false` | 状态栏按钮显示最近一条文本（截断 20 字） | `:149-151` |
| 4 | HStack：Toggle + Picker | ShowSearchField / 显示搜索框 + 搜索可见性选择器 | `true` / `.always` | Picker 选项 Always/始终、During search/在搜索过程中；`showSearch == false` 时 disabled | `:152-166` |
| 5 | Toggle | ShowTitleBeforeSearchField / 在搜索框前显示应用名 | `true` | 显示 "Manis Pocket" | `:167-169` |
| 6 | Toggle | ShowApplicationIcons / 显示应用程序图标 | `true` | 控制列表行左侧 15×15 应用图标 | `:170-172` |
| 7 | Toggle | ShowFooter / 显示底部菜单 | `true` | 隐藏 footer | `:174-176` |
| 8 | Text（small、gray） | `OpenPreferencesWarning` = "⚠️ Press ⌘, (command+comma) to open preferences when footer is hidden." /「⚠️ 隐藏底部菜单时仍可按 ⌘, 打开偏好设置。」 | — | `opacity(showFooter ? 0 : 1)`，即**只在 footer 隐藏时可见** | `:177-180` |

### 6.4 Pins（固定）

`ManisPocket/Settings/PinsSettingsPane.swift`。整体不是 `Settings.Container`，而是一个自定义 VStack：

```
VStack(alignment: .leading)
 ├─ Table(pinnedItems, selection:)
 │    ├─ TableColumn("Key"/"键位", width: 60)    → item.pin（单个字母）
 │    ├─ TableColumn("Alias"/"标题")             → decorator.title
 │    └─ TableColumn("Content"/"内容")           → Clipboard.getText(item) ?? ""，lineLimit(1)，truncationMode(.tail)
 ├─ Text("PinCustomizationDescription", gray, small)
.padding()
.frame(width: SettingsLayout.paneWidth)   // 610
.frame(minHeight: 400)
```

- 双击单元格可编辑（系统 Table 行为）；`onDeleteCommand` 删除选中项（`:37-42`）。
- 描述文案：`PinCustomizationDescription` = "You can customize the hotkey, title, and content of any pinned item. To edit, double-click the item and enter a new value.\nPlease note that only plain text can be changed." /「您可以自定义任何固定项目的热键、标题和内容。\n要编辑，请双击该项目并输入新值。」
- 来源：`:15-51`、`ManisPocket/Settings/en.lproj/PinsSettings.strings:1-7`
- 其它已定义但本 pane 未使用的 key：`ContentIsNotText`、`RichTextEditWarning`（`PinsSettings.strings:5-6`）——移植时可不实现。

### 6.5 Ignore（忽略）

`ManisPocket/Settings/IgnoreSettingsPane.swift:3-23`：`TabView` + 3 个 tab，`.padding()`，`frame(width: 610)`，`frame(minHeight: 400)`。

Tab 文案（英 / 中）：`ApplicationsTab` = Applications/忽略应用；`PasteboardTypesTab` = Pasteboard types/忽略剪贴板类型；`RegexpTab` = Regular expressions/正则表达式（`IgnoreSettings.strings:2,5,8`）。

#### 6.5.1 Applications tab（`IgnoreApplicationsSettingsView`）

```
VStack(alignment: .leading)
 ├─ List(selection:)
 │    每行：Label(title: 应用显示名.padding(.horizontal, 5), icon: 应用图标)
 │          .frame(height: 32).padding(.horizontal, 5)
 │    找不到应用时：icon 用 "questionmark.circle".imageScale(.large)，title 用 bundleId
 ├─ HStack
 │    ├─ ControlGroup { Button("plus"); Button("minus") }.frame(width: 50)
 │    │    （fileImporter 仅允许 .application，默认目录 /Applications）
 │    └─ Toggle("IgnoredAllAppsExceptListed" / 忽略列表之外的所有应用程序)  → Defaults[.ignoreAllAppsExceptListed]，默认 false
 └─ Text(IgnoredAppsDescription, gray, small, 多行)
.padding()
```

- 添加：选择 .app → 取 bundleIdentifier → 去重后 append 到 `Defaults[.ignoredApps]`（默认 `[]`）。
- 删除：`onDeleteCommand` 或 minus 按钮。
- 来源：`ManisPocket/Settings/IgnoreSettingsPane/IgnoreApplicationsSettingsView.swift:10-80`、`ManisPocket/Extensions/Defaults.Keys+Names.swift:23,27`。

#### 6.5.2 Pasteboard types tab（`IgnorePasteboardTypesSettingsView`）

- `List` 中每行是一个**可直接编辑的 TextField**（无边框），提交时：从集合移除旧值、插入新值（`:13-27`）。
- `ControlGroup { plus; minus }`，`frame(width: 50)`；plus 插入占位值 `"xxx.yyy.zzz"` 并聚焦（`:35-38`）。
- 右侧 `Spacer()` + `Button("IgnoredPasteboardTypesReset"/"重置")` → `Defaults.reset(.ignoredPasteboardTypes)`（`:45-52`）。
- 说明 `IgnoredPasteboardTypesDescription`。
- 默认值 5 项：`Pasteboard generator type`、`com.agilebits.onepassword`、`com.typeit4me.clipping`、`de.petermaurer.TransientPasteboardType`、`net.antelle.keeweb`（`ManisPocket/Extensions/Defaults.Keys+Names.swift:28-37`）。

#### 6.5.3 Regexp tab（`IgnoreRegexpsSettingsView`）

- 与 6.5.2 同构，但数据是 `[String]`（有序数组），plus 插入模板 `"^[a-zA-Z0-9]{50}$"` 并聚焦（`:31-34`）。
- 无 Reset 按钮。
- 默认 `[]`（`ManisPocket/Extensions/Defaults.Keys+Names.swift:26`）。

### 6.6 Advanced（高级）

`ManisPocket/Settings/AdvancedSettingsPane.swift:5-43`，`VStack(alignment: .leading)`，`.padding()`，`frame(width: 610)`。

| # | 控件 | 标签（英 / 中） | 默认 | 说明 | 行号 |
|---|---|---|---|---|---|
| 1 | Toggle | TurnOff / 暂停 | `false`（`ignoreEvents`） | 暂停记录 | `:7-9` |
| 2 | Text（gray、small、多行） | `TurnOffDescription` = "Temporarily ignore all new copies.\nYou are likely to use it programmatically and disable application while copying sensitive data." /「暂停记录新的剪贴板项目。…」 | — | 说明 | `:10-13` |
| 3 | Text（**monospaced 11pt**、gray、small、`padding(.vertical, 2)`） | `TurnOffShellScript` = 两行 `defaults write com.kaigedong.manispocket ignoreEvents true/false` | — | 命令行暂停 | `:14-19` |
| 4 | Text（gray、small） | `TurnOffViaMenuIconDescription` = "You can also click the menu icon with ⌥ pressed.\nTo ignore only the next copy, click with ⌥⇧." | — | 说明 | `:20-23` |
| 5 | Text（monospaced 11pt） | `TurnOffNextShellScript` = `defaults write com.kaigedong.manispocket ignoreOnlyNextEvent true` | — | 只忽略下一次 | `:24-29` |
| 6 | **Divider** | — | — | 无内边距 | `:31` |
| 7 | Toggle（help: ClearHistoryOnQuitTooltip） | ClearHistoryOnQuit / 退出时清除历史记录 | `false`（`clearOnQuit`） | 退出时清空未固定项 | `:33-35` |
| 8 | Toggle（help: ClearSystemClipboardTooltip） | ClearSystemClipboard / 同时清除系统剪贴板 | `false`（`clearSystemClipboard`） | 清历史时也清系统剪贴板 | `:37-39` |

### 6.7 Sync（同步）

`ManisPocket/Settings/SyncSettingsPane.swift`。使用 `Settings.Container(contentWidth: 550)`，其中 `sectionContentWidth = 550 - 150 = 400`（`:6`）。**本节所有文案都是硬编码英文（不走本地化表）**。

**Section 1「Enable:」** — `:25-44`
| # | 控件 | 标签 | 默认 | 作用 |
|---|---|---|---|---|
| 1 | Toggle | "Enable Clipboard Sync" | `false`（`syncEnabled`） | 开 → `SyncBridge.start()`；关 → `stop()`（`:29-35`） |
| 2 | Toggle | "Allow Discovery" | `false`（`syncDiscoverable`） | mDNS 可被发现 |
| 3 | Text（`.caption`、secondary、`frame(width: 400, alignment: .leading)`、可换行） | "When enabled, other devices on the same network can find this device automatically via mDNS." | — | 说明 |

**Section 2「This Device:」** — `:46-49`
| 控件 | 规格 |
|---|---|
| TextField（placeholder "Device Name"，`frame(width: 200)`） | 绑定 `Defaults[.syncDeviceName]`，默认 `Host.current().localizedName ?? "Mac"` |

**Section 3「Paired Devices:」** — `:51-53,147-190`

- 空态：`Text("No paired devices").foregroundStyle(.secondary).controlSize(.small)`（`:149-152`）。
- 每个已配对设备一行 `pairedDeviceRow`，`.frame(width: 400)`（`:160-190`）：

```
HStack
 ├─ Text(device.icon)                       // emoji
 ├─ VStack(alignment: .leading)
 │    ├─ Text(device.nickname).lineLimit(1)
 │    └─ HStack(spacing: 4)
 │         Circle().fill(isConnected ? .green : .gray).frame(width: 6, height: 6)
 │         Text(isConnected ? "Connected" : "Offline").font(.caption2).foregroundStyle(.secondary)
 ├─ Spacer()
 └─ [仅 device.isAdmin]
      Button(pencil 图标, .borderless, .small)  → 打开编辑弹窗
      Button("Unpair", .bordered, .small)      → SyncBridge.unpair + core.removePairedPeer + 重载
```

**Section 4「Discovered Devices:」（bottomDivider）** — `:55-72,97-145`

- 标签行右侧有一个刷新按钮：`HStack(spacing: 6) { Text("Discovered Devices"); Button(arrow.clockwise, .borderless, .small) }`，点击 `SyncBridge.refreshDiscovery()` 并清空本地列表（`:58-68`）。
- 空态（`:99-108`）：
  - `Text("No devices found.").foregroundStyle(.secondary).controlSize(.small)`
  - `Text("Make sure both devices are on the same network and \"Local Network\" permission is enabled in System Settings → Privacy & Security.").font(.caption2).foregroundStyle(.tertiary)`，可换行
- 有设备时每行：`HStack { Text(peer.displayName); Spacer(); Button("Pair", .bordered, .small) }`（`:110-118`）。
- **分隔线**（`:121`），然后手动连接行（`:123-138`）：
  - `TextField("IP:Port", frame(width: 200))`
  - `Button("Connect", .bordered, .small)`，`manualAddress.isEmpty` 时 disabled；点击后状态文案置为 "Connecting..."，3 秒后若仍是它则清除
- **状态行**（`:140-144`）：`Text(status)`，`.font(.caption)`；文本含 `fail` 或 `error` → `.red`，否则 `.secondary`。来自 `.syncError` 通知时 **5 秒**后自动消失（`:89-94`）。

**配对对话框（sheet）** — `:217-246`

```
VStack(spacing: 16)
 ├─ Text("Pairing Request").font(.headline)
 ├─ Text("Pairing with \"<displayName>\".")
 ├─ Text("Confirm this PIN on both devices:")
 ├─ HStack(spacing: 8)  // 每个字符一个方块
 │    Text(char).font(.system(.title, design: .monospaced))
 │              .frame(width: 32, height: 40)
 │              .background(Color.primary.opacity(0.05))
 │              .clipShape(RoundedRectangle(cornerRadius: 4))
 └─ HStack { Button("Cancel") [cancelAction]; Button("Confirm") [defaultAction] }
.padding(24)
.frame(width: 340)
```

**编辑设备弹窗（sheet）** — `:192-215`

```
VStack(spacing: 16)
 ├─ Text("Edit Device").font(.headline)
 ├─ HStack { Text("Icon:"); TextField(frame(width: 40)).font(.title2) }   // emoji
 ├─ HStack { Text("Name:"); TextField("Nickname", frame(width: 200)) }
 └─ HStack { Button("Cancel") [cancelAction]; Button("Save") [defaultAction] }
.padding(24)
.frame(width: 320)
```

- Save → `core.savePairedPeer(peerId:displayName:isAdmin: false)`，重载列表。
- 默认编辑值：`editingIcon = "💻"`、`editingNickname` 取当前 nickname（`:15-16,176`）。

---

## 7. 键盘交互

### 7.1 三层分工

| 层 | 实现 | 作用范围 | 来源 |
|---|---|---|---|
| ① 全局快捷键（Carbon RegisterEventHotKey） | `KeyboardShortcuts` 包 | 系统级，App 不在前台也生效 | `ManisPocket/Extensions/KeyboardShortcuts.Name+Shortcuts.swift:4-7` |
| ② 面板内本地监听 | `NSEvent.addLocalMonitorForEvents([.flagsChanged, .keyDown])` | 面板存在时，用于"循环模式" | `ManisPocket/Observables/Popup.swift:60-67` |
| ③ 视图层按键分发 | `KeyHandlingView.onKeyPress` + `KeyChord` | 面板内所有按键的语义映射 | `ManisPocket/Views/KeyHandlingView.swift:14-180`、`ManisPocket/KeyChord.swift:67-153` |

### 7.2 全局快捷键（默认值）

| 名称 | 默认组合 | 是否全局启用 | 作用 | 来源 |
|---|---|---|---|---|
| `popup` | **⇧⌘C** | ✅ 启用 | 打开/关闭面板；按住修饰键连按可循环选择 | `KeyboardShortcuts.Name+Shortcuts.swift:4`、`ManisPocket/Observables/Popup.swift:52` |
| `togglePreview` | **⌃Space** | ✅ 启用（未被禁用） | 切换预览侧栏 | `KeyboardShortcuts.Name+Shortcuts.swift:7`、`ManisPocket/AppDelegate.swift:199-201` |
| `pin` | **⌥P** | ❌ 启动即 `KeyboardShortcuts.disable` | 仅面板内生效 | `KeyboardShortcuts.Name+Shortcuts.swift:5`、`ManisPocket/AppDelegate.swift:199-212` |
| `delete` | **⌥⌫** | ❌ 启动即 disable | 仅面板内生效 | `KeyboardShortcuts.Name+Shortcuts.swift:6`、`ManisPocket/AppDelegate.swift:199-212` |

> 注意 .pin/.delete 的 Recorder 在设置里仍可改，改动后会被再次 disable（监听 `KeyboardShortcuts_shortcutByNameDidChange`）（`ManisPocket/AppDelegate.swift:203-211`）。

### 7.3 循环选择模式（hold-modifier cycle）

状态机 `Popup.state ∈ {toggle, cycle, opening}`（`ManisPocket/Observables/Popup.swift:6-16`）：

1. 面板关闭时按下 ⇧⌘C → `open()` + `state = .opening`，禁用全局快捷键（避免重复触发），转由本地监听器处理（`:113-119`）。
2. `.opening` 状态下再按下主键（keyCode 匹配 `popup` 的键）→ `state = .cycle`，并 `highlightNext(allowCycle: true)`（`:146-154`）。
3. `.cycle` 状态下继续按主键 → 一直 `highlightNext(allowCycle: true)`（到底后回绕到首项）（`:151-154`）。
4. `.cycle` 状态下**松开所有修饰键** → 延时一帧后 `AppState.select()`（复制并关闭）（`:165-172`）。
5. `.opening` 状态下松开修饰键但没按过主键 → 转 `.toggle`，面板保持打开（`:175-178`）。
6. `.toggle` 状态下再按一次完整组合 → 关闭（`:156-159`）。

### 7.4 面板内按键 → 语义（完整表）

来自 `KeyChord.init(_ key:_ modifierFlags:)`（`ManisPocket/KeyChord.swift:91-153`）。符号：`⌘`command、`⌥`option、`⌃`control、`⇧`shift、`⌫`delete、`↩`return、`⌤`keypadEnter、`␣`space、`⇞`pageUp、`⇟`pageDown。

| 语义 | 触发键（全部等价） | 行为 | 源码行 |
|---|---|---|---|
| `clearHistory` | `⌥⌘⌫` | 清空未固定项（走确认） | `KeyChord.swift:93-94`、`KeyHandlingView.swift:31-44` |
| `clearHistoryAll` | `⇧⌥⌘⌫` | 清空全部（走确认） | `KeyChord.swift:95-96`、`KeyHandlingView.swift:45-58` |
| `clearSearch` | `⌃U` | 清空搜索框 | `KeyChord.swift:97-98`、`KeyHandlingView.swift:59-61` |
| `deleteCurrentItem` | `⌥⌫`（= `delete` 快捷键当前值） | 删除选中项；若选中的是贴栈则 `removePasteStack()` | `KeyChord.swift:99-100`、`KeyHandlingView.swift:62-68` |
| `deleteOneCharFromSearch` | `⌃H` | 聚焦搜索框并删 1 个字符 | `KeyChord.swift:101-102`、`KeyHandlingView.swift:69-72` |
| `deleteLastWordFromSearch` | `⌃W` | 删最后一个词并补一个空格 | `KeyChord.swift:103-104`、`KeyHandlingView.swift:73-82` |
| `moveToNext` | `↓`、`⌃N`、`⌃J` | 下一个（列表到底进入 footer 首项） | `KeyChord.swift:108-111`、`KeyHandlingView.swift:83-89` |
| `moveToLast` | `⌘↓`、`⌥↓`、`⌃⌥N`、`⇟` | 跳到最后一项 | `KeyChord.swift:116-119`、`KeyHandlingView.swift:90-96` |
| `moveToPrevious` | `↑`、`⌃P`、`⌃K` | 上一个 | `KeyChord.swift:123-126`、`KeyHandlingView.swift:97-103` |
| `moveToFirst` | `⌘↑`、`⌥↑`、`⌃⌥P`、`⇞` | 跳到第一项 | `KeyChord.swift:131-134`、`KeyHandlingView.swift:104-110` |
| `extendToNext` | `⇧↓`、`⌃⇧N`（**仅多选开启时**） | 扩展选区向下 | `KeyChord.swift:105-107`、`KeyHandlingView.swift:111-119` |
| `extendToLast` | `⇧⌘↓`、`⇧⌥↓`、`⇧⌥⌃N` | 扩展选区到末项 | `KeyChord.swift:112-115`、`KeyHandlingView.swift:120-128` |
| `extendToPrevious` | `⇧↑`、`⌃⇧P` | 扩展选区向上 | `KeyChord.swift:120-122`、`KeyHandlingView.swift:129-137` |
| `extendToFirst` | `⇧⌘↑`、`⇧⌥↑`、`⇧⌥⌃P` | 扩展选区到首项 | `KeyChord.swift:127-130`、`KeyHandlingView.swift:138-146` |
| `openPreferences` | `⌘,` | 打开设置窗口 | `KeyChord.swift:139-140`、`KeyHandlingView.swift:147-149` |
| `pinOrUnpin` | `⌥P`（= `pin` 快捷键当前值） | 固定/取消固定选中项 | `KeyChord.swift:135-136`、`KeyHandlingView.swift:150-152` |
| `selectCurrentItem` | `↩`、`⌤` | 确认选中（复制/粘贴并关闭） | `KeyChord.swift:141-143`、`KeyHandlingView.swift:153-155` |
| `editCurrentItem` | `⌘E` | 进入行内编辑 | `KeyChord.swift:137-138`、`KeyHandlingView.swift:156-158` |
| `close` | `Esc` | 关闭面板 | `KeyChord.swift:144-145`、`KeyHandlingView.swift:159-161` |
| `togglePreview` | `⌃Space`（= `togglePreview` 快捷键当前值） | 切换预览侧栏 | `KeyChord.swift:146-147`、`KeyHandlingView.swift:162-164` |
| `ignored` | 任意带 `⌘`/`⌃`/`⌥` 但未匹配上述的键 | 吞掉，不落回系统 | `KeyChord.swift:148-149` |
| `unknown` | 其余（纯字符键） | 落到"快捷选择"逻辑 | `KeyChord.swift:150-151` |

**快捷选择（数字/字母直选）**：若 `history.pressedShortcutItem` 非空（当前按下的键匹配某行显示的快捷键字符），先 `navigator.select(item:)`，再延时 **50ms** `history.select(item:)`（即复制并关闭）（`ManisPocket/Views/KeyHandlingView.swift:169-176`、`ManisPocket/Observables/History.swift:88-103`）。

**前置守卫（必须移植）**：
- 若有输入法候选窗（`hasMarkedText()`）→ 返回 `.ignored`（`ManisPocket/Views/KeyHandlingView.swift:20-28`）。
- 若系统字符选择器打开（`NSApp.characterPickerWindow != nil`）→ 方向键语义返回 `.ignored`（`:84-86,91-93,98-100,105-107,112-114,121-123,130-132,139-141`）。
- 多选未开启时 `extend*` 全部返回 `.ignored`（`:115-117` 等）。

**修饰键归一化**：`modifierFlags ∩ .deviceIndependentFlagsMask - [.capsLock, .numericPad, .function]`；若"当前布局下 ⌘ 会切到 QWERTY"则用 `Key(QWERTYKeyCode:)` 解析（`ManisPocket/KeyChord.swift:73-80`）。

### 7.5 选择项时的"动作"矩阵（最容易做错的部分）

`History.select(item)`：无修饰键时直接 `close() + copy(removeFormatting: Defaults[.removeFormattingByDefault])`，若 `pasteByDefault` 再 `paste()`；有修饰键时按 `HistoryItemAction` 分派（`ManisPocket/Observables/History.swift:365-397`）。

`HistoryItemAction` 由修饰键 + 两个默认值决定（`ManisPocket/HistoryItemAction.swift:10-37`）：

| pasteByDefault | removeFormattingByDefault | `⌘` | `⌥` | `⌥⇧` | `⇧⌘` |
|---|---|---|---|---|---|
| **false** | **false**（默认） | copy | **paste**（复制+粘贴） | **pasteWithoutFormatting** | 未定义 |
| false | true | copy | pasteWithoutFormatting | paste | 未定义 |
| true | false | paste | copy | 未定义 | pasteWithoutFormatting |
| true | true | pasteWithoutFormatting | copy | 未定义 | paste |

三种动作的实际差异：
- `copy`：写剪贴板 + `popup.close()`。
- `paste`：`copy` + 模拟 ⌘V（CGEvent）。
- `pasteWithoutFormatting`：`copy` 时只保留 `string`（+`fileURL`）类型，再模拟 ⌘V。

（`ManisPocket/Clipboard.swift:85-125,127-147,461-475`）

### 7.6 鼠标交互

| 交互 | 行为 | 来源 |
|---|---|---|
| 单击行 | 有 `⌘` 且多选开启 → `addToSelection`；否则 `history.select(item)`（复制并关闭） | `ManisPocket/Views/HistoryItemView.swift:128-136` |
| 悬停行 | 若不在键盘导航且不在多选中 → `selectWithoutScrolling(id:)` 即时改变选中（不滚动）；否则记下 `hoverSelectionWhileKeyboardNavigating`，待退出键盘导航时生效 | `ManisPocket/Views/HoverSelectionModifier.swift:8-16`、`ManisPocket/Observables/NavigationManager.swift:59-69` |
| 任意鼠标移动 | `navigator.isKeyboardNavigating = false`（此后悬停即选中） | `ManisPocket/Views/ContentView.swift:45-47` |
| 悬停行内按钮 | 显示按钮组（见 §2.3） | `ManisPocket/Views/HistoryItemView.swift:67-105` |
| 悬停 footer 项 | 若预览打开则关闭预览 | `ManisPocket/Views/FooterItemView.swift:13-17` |
| 拖动窗口 | 背景任意处拖动；`onEnded` 时保存归一化位置 | `ManisPocket/FloatingPanel.swift:58-61,108-117` |
| 拖动预览分隔条 | 调整预览宽度（见 §9.6） | `ManisPocket/Views/SlideoutView.swift:51-95` |

---

## 8. 视觉参数总表

### 8.1 间距 / 尺寸

| 项 | 值 | 来源 |
|---|---|---|
| 窗口默认尺寸 | 450 × 800（宽会被用户拖动改变，高是上限） | `ManisPocket/Extensions/Defaults.Keys+Names.swift:61` |
| 窗口圆角 | 9（`[26]` 12） | `ManisPocket/FloatingPanel.swift:63` |
| 行高（最小） | 22（`[26]` 24） | `ManisPocket/Observables/Popup.swift:34-38` |
| 行圆角 | 4（`[26]` 7） | `ManisPocket/Observables/Popup.swift:28-32` |
| 行左内边距（无图标） | 10 | `ManisPocket/Views/ListItemView.swift:62` |
| 行左内边距（有图标） | 4 + 15（图标）+ 5 | `ManisPocket/Views/ListItemView.swift:57,62` |
| 应用图标 | 15 × 15，左 4、上下 5 | `ManisPocket/Views/ListItemView.swift:54,57-58` |
| 筛选栏图标 | 18 × 18，圆角 4 | `ManisPocket/Views/AppFilterBar.swift:77,89` |
| 颜色片 | 12 × 12 | `ManisPocket/ColorImage.swift:10-12` |
| 缩略图上限 | 340 × `imageMaxHeight`(40)，等比不放大 | `ManisPocket/Observables/HistoryItemDecorator.swift:17-19` |
| 快捷键提示右内边距 | 10 | `ManisPocket/Views/ListItemView.swift:107` |
| 快捷键提示条目间距 | 5 | `ManisPocket/Views/ListItemView.swift:83` |
| 快捷键字符宽度 | 12，修饰符间距 1 | `ManisPocket/Views/KeyboardShortcutView.swift:19,23` |
| hover 按钮预留宽度 | 56 | `ManisPocket/Views/HistoryItemView.swift:63` |
| hover 按钮 | 24 × 22，圆角 5，间距 2，右 8 | `ManisPocket/Views/ActionIconButton.swift:14,17`、`ManisPocket/Views/HistoryItemView.swift:68,103` |
| Header 上内边距 | 5 | `ManisPocket/Views/HeaderView.swift:53` |
| Header 左右内边距 | 10 | `ManisPocket/Views/HeaderView.swift:54` |
| 标题左内边距 | 5 | `ManisPocket/Views/ListHeaderView.swift:19` |
| 搜索框高度 | 23 | `ManisPocket/Views/SearchFieldView.swift:14` |
| 搜索框内图标 | 11 × 11，左 5 / 右 5 | `ManisPocket/Views/SearchFieldView.swift:17-20,34-37` |
| 工具栏按钮高度 | 23 | `ManisPocket/Views/ToolbarView.swift:41` |
| 筛选栏高度 | 26 | `ManisPocket/Views/AppFilterBar.swift:53` |
| 筛选栏图标间距 / 内边距 | 6 / 4 | `ManisPocket/Views/AppFilterBar.swift:19,51` |
| 筛选栏分隔竖线 | 1 × 16，`primary @ 0.15` | `ManisPocket/Views/AppFilterBar.swift:33-35` |
| 列表上下 padding | 6 / 6（footer 隐藏时下为 5） | `ManisPocket/Views/HistoryListView.swift:41-49` |
| 分隔线左右 padding | 6 | `ManisPocket/Observables/Popup.swift:21` |
| Footer 底内边距 | 5 | `ManisPocket/Views/FooterView.swift:59` |
| 预览最小高度 | 150 | `ManisPocket/Observables/Popup.swift:24` |
| 预览最小宽度 / 默认宽度 | 200 / 400 | `ManisPocket/Observables/SlideoutController.swift:73,77,89`、`ManisPocket/Extensions/Defaults.Keys+Names.swift:64` |
| 内容区最小宽度 | 200 | `ManisPocket/Observables/SlideoutController.swift:73` |

### 8.2 颜色 / 不透明度

| 项 | 值 | 来源 |
|---|---|---|
| 搜索框底 | `Color.secondary` @ **0.1** | `ManisPocket/Views/SearchFieldView.swift:12-13` |
| 搜索放大镜 | opacity 0.8 | `ManisPocket/Views/SearchFieldView.swift:20` |
| 搜索清除按钮 | opacity 0.9 | `ManisPocket/Views/SearchFieldView.swift:39` |
| 行选中底 | `Color.accentColor` @ **0.8** | `ManisPocket/Views/ListItemView.swift:115` |
| 行选中文字 | 白 | `ManisPocket/Views/ListItemView.swift:112` |
| 行非选中底 | 透明（源码 0.001 白，仅 workaround） | `ManisPocket/Views/ListItemView.swift:115` |
| 行内容默认色 | `.primary` | `ManisPocket/Views/ListItemView.swift:112` |
| 编辑态底 | `accentColor` @ **0.15** | `ManisPocket/Views/HistoryItemView.swift:222` |
| 复制气泡底 | `Color.green` @ **0.85**，文字白 | `ManisPocket/Views/HistoryItemView.swift:113-117` |
| 筛选图标变暗 | opacity **0.45** | `ManisPocket/Views/AppFilterBar.swift:79` |
| 筛选图标选中底 | `accentColor` @ **0.2** | `ManisPocket/Views/AppFilterBar.swift:83` |
| 筛选图标 hover 底 | `primary` @ **0.08** | `ManisPocket/Views/AppFilterBar.swift:86` |
| 筛选图标选中描边 | `accentColor` @ **0.8**，1px | `ManisPocket/Views/AppFilterBar.swift:92` |
| 设备图标排除底 | `red` @ **0.1** | `ManisPocket/Views/DeviceFilterIcon.swift:16` |
| 设备图标排除描边 | `red` @ **0.5**，1px | `ManisPocket/Views/DeviceFilterIcon.swift:21` |
| 设备图标排除整体 | opacity **0.3** | `ManisPocket/Views/DeviceFilterIcon.swift:23` |
| 删/复制按钮 hover 底（浅） | `red`/`blue` @ **0.15** | `ManisPocket/Views/ActionIconButton.swift:39,42` |
| 删/复制按钮 hover 底（选中行） | `red`/`blue` @ **0.5** | `ManisPocket/Views/ActionIconButton.swift:39,42` |
| 按钮静止前景（选中行） | `white` @ **0.6** | `ManisPocket/Views/ActionIconButton.swift:52` |
| 快捷键提示 | opacity **0.7** | `ManisPocket/Views/KeyboardShortcutView.swift:26` |
| 设置页说明文字 | `.gray` + `.controlSize(.small)` | 各 pane（如 `GeneralSettingsPane.swift:103-104`） |
| 贴栈未选中底 | `NSColor.tertiarySystemFill` @ 0.8 + `.thinMaterial` | `ManisPocket/Views/PasteStackView.swift:86-91` |
| 贴栈选中底 | `accentColor` @ 0.8 | `ManisPocket/Views/PasteStackView.swift:88` |

### 8.3 修饰符符号顺序

`NSEvent.ModifierFlags.description` 固定顺序 **⌃ ⌥ ⇧ ⌘**（再追加 🌐 若含 fn）（`ManisPocket/Extensions/ModifierFlags+Description.swift:6-30`）。

### 8.4 快捷键提示的显隐规则

`KeyShortcut.isVisible(all:pressed:)`（`ManisPocket/KeyShortcut.swift:31-47`）：

```
若该行只有一个快捷键 → 总是可见
若 modifierFlags == [⌘] 且当前无修饰键按下 → 可见
若 modifierFlags == [⌘] 且当前有修饰键按下，且该行没有别的快捷键与当前修饰键完全相同 → 可见
否则 → 可见 iff modifierFlags == 当前按下集合
```

含义：静止时显示 `⌘X` 那套；按住 ⌥ 时显示 `⌥X`；按住 ⌥⇧ 时显示 `⌥⇧X`。不可见时宽度收为 0（不占位）。

### 8.5 动画

| 场景 | 曲线 / 时长 | 来源 |
|---|---|---|
| 窗口高度变化 | 0.2s（`NSAnimationContext`，默认 ease） | `ManisPocket/FloatingPanel.swift:95-98` |
| 预览侧栏开合 | `easeInOut` **0.25s**（`SlideoutController.animationDuration`） | `ManisPocket/Observables/SlideoutController.swift:68,163,188,194` |
| 搜索显隐 | `easeInOut` **0.2s** | `ManisPocket/Views/ContentView.swift:60` |
| 列表变化 / 贴栈变化 / lead 变化 | SwiftUI `.default` **speed(3)** → 约 0.12–0.2s | `ManisPocket/Views/ContentView.swift:36-40`、`ManisPocket/Views/HeaderView.swift:55` |
| 行 hover 展开按钮 | `easeInOut` **0.15s** | `ManisPocket/Views/HistoryItemView.swift:64` |
| 按钮 hover 变色 | `easeInOut` **0.1s** | `ManisPocket/Views/ActionIconButton.swift:22` |
| 复制气泡 / 按钮组出现 | `.opacity` transition（默认时长） | `ManisPocket/Views/HistoryItemView.swift:104,119` |
| 预览自动打开延迟 | `Defaults[.previewDelay]` 默认 **500ms** | `ManisPocket/Observables/SlideoutController.swift:232`、`Defaults.Keys+Names.swift:47` |
| 搜索节流 | **200ms** | `ManisPocket/Observables/History.swift:105` |
| 滚动到选中项延迟 | **10ms** | `ManisPocket/Views/HistoryListView.swift:109,136` |
| 快捷选择后复制延迟 | **50ms** | `ManisPocket/Views/KeyHandlingView.swift:172` |
| 复制气泡停留 | **1.5s** | `ManisPocket/Views/HistoryItemView.swift:90` |
| 剪贴板轮询间隔 | `0.5s`（`clipboardCheckInterval`） | `ManisPocket/Extensions/Defaults.Keys+Names.swift:16` |
| 同步错误提示停留 | **5s** | `ManisPocket/Settings/SyncSettingsPane.swift:92` |

---

## 9. 已知状态与功能表现

### 9.1 标题生成（含 OCR）

**文本项标题**（`ManisPocket/Clipboard.swift:287-309`）：
1. 若内容含图片 → 标题返回空字符串（待 OCR 填充）。
2. 否则 `getPreviewableText()` 取文本并 `shortened(to: 1000)`。
3. `showSpecialSymbols == true`（默认）时：
   - 行首空格 → 每个空格替换为 `·`（正则 `^ +`）
   - 行尾空格 → 每个空格替换为 `·`（正则 ` +$`）
   - `\n` → `⏎`，`\t` → `⇥`
4. `showSpecialSymbols == false` 时改为 `trimmingCharacters(in: .whitespacesAndNewlines)`。

`getPreviewableText` 优先级（`ManisPocket/Clipboard.swift:350-385`）：`fileURL`（多个用 `\n` 连接、去百分号编码）→ `string` → `rtf`（转纯文本）→ `html`（转纯文本）→ `item.title`。

**图片项 OCR**（`ManisPocket/Models/HistoryItem.swift:241-267`）：
- 用 Vision `VNRecognizeTextRequest`，`recognitionLevel = .fast`。
- 结果按观察顺序 `topCandidates(1).first.string`，**用 `\n` 拼接**，直接赋给 `title`。
- UI 表现：图片行没有文字（显示缩略图），但 OCR 文本会参与搜索匹配和预览面板的标题来源。**OCR 是异步的**：先以空标题入列表，识别完成后标题才更新。
- 移植提示：Linux 端没有 Vision，需要用 `tesseract`/`ocrs` 或直接放弃 OCR（降级为无标题、仅能按时间/来源检索）。

### 9.2 图片预览（侧栏）

`PreviewItemView`（`ManisPocket/Views/PreviewItemView.swift:4-99`）：

- 结构：`VStack(alignment: .leading, spacing: 0)`：
  1. 图片项 → 图片（`aspectRatio(contentMode: .fit)`，圆角 **5**），加载中显示 `Color.gray.opacity(0.3)` 占位 + `ProgressView`；失败显示 `photo.badge.exclamationmark`（multicolor）。
  2. 文本项 → `ScrollView { Text(item.text).font(.body) }`。
  3. `Spacer(minLength: 0)`。
  4. `Divider().padding(.vertical)`。
  5. 元信息行（`HStack(spacing: 3)`）：
     - `Application:` + 11×11 应用图标 + 应用名
     - `Dimensions:` + `<宽>×<高>`（像素）
     - `First copy time:` + 日期 + 时间
     - `Last copy time:` + 日期 + 时间
     - `Number of copies:` + 数字
  6. 整体 `.controlSize(.small)`。
- 预览图尺寸 = 屏幕 `visibleFrame` 大小（`ManisPocket/Observables/HistoryItemDecorator.swift:13-15`）。
- 侧栏容器：`VStack { ToolbarView(); PreviewItemView(..) }`，`.padding(.horizontal)`、`.padding(.bottom)`、`.padding(.top, 5)`（`ManisPocket/Views/SlideoutContentView.swift:7-21`）。
- `ToolbarView` 在选中项非空时右侧显示：pin/pin.slash 按钮 + trash 按钮（`HStack` + `Spacer` 靠右），贴栈被选中时显示 `stop` 按钮（`ManisPocket/Views/ToolbarView.swift:84-129`）。

**预览开合规则**（`ManisPocket/Observables/SlideoutController.swift`）：
- 手动切换（⌃Space 或右侧按钮）：`togglePreview(trigger: .manual)`；关闭时置 `autoOpenSuppressed = true`，避免立刻被自动打开（`:148-160`）。
- 自动打开：选中项变化 → `startAutoOpen()`，延时 `previewDelay` 后若仍关闭则 `togglePreview(trigger: .autoOpen)`（`:224-239`、`ManisPocket/Observables/NavigationManager.swift:37-48`）。
- 位置：新增宽度会向右扩；若超出屏幕右边界则改为**向左**扩展（`placement = .left`）（`:128-136`）。
- 关闭（失焦）时 `preview.state = .closed`（`ManisPocket/FloatingPanel.swift:203`）。

### 9.3 Append 模式（连按两次 ⌘C）

- 开关 `Defaults[.appendModeEnabled]`（默认 false）；时间窗 `Defaults[.appendModeTimeWindow]`（默认 **1.0 秒**）（`ManisPocket/Extensions/Defaults.Keys+Names.swift:17-18`）。
- 触发条件（`ManisPocket/Clipboard.swift:257-275`）：`changeCountDelta == 2`（一次复制造成两格变化）**或**（文本与上一次完全相同 **且** 距上次变化 ≤ 时间窗）。
- 合并行为（`ManisPocket/Observables/History.swift:206-242`）：把新文本追加到最近一条**未固定且内容不同**的项的 `string` 内容后：`combinedText = existing + "\n" + new`，`lastCopiedAt` 更新，`numberOfCopies += 1`，标题重新生成，并回写剪贴板。
- UI 表现：**不新增行**，原行标题变成两行文本（`\n` 会被 `showSpecialSymbols` 渲染成 `⏎`），`numberOfCopies` 增加。

### 9.4 忽略规则（UI 上表现为"复制了但没进列表"）

| 规则 | 条件 | 默认值 | 来源 |
|---|---|---|---|
| 应用黑名单 | `ignoredApps.contains(bundleId)`（当 `ignoreAllAppsExceptListed == false`） | `[]` | `ManisPocket/Clipboard.swift:401-407` |
| 应用白名单模式 | `ignoreAllAppsExceptListed == true` 时，**不在**列表中的应用被忽略 | `false` | 同上 |
| 剪贴板类型黑名单 | 命中 `ignoredPasteboardTypes` 或内置 `autoGenerated/concealed/transient` | 5 项（见 §6.5.2） | `ManisPocket/Clipboard.swift:393-399` |
| 启用类型过滤 | `enabledPasteboardTypes`（Storage 页三个开关） | 全部 8 类 | `ManisPocket/Extensions/Defaults.Keys+Names.swift:19-21` |
| 正则 | `ignoreRegexp` 任一正则在 `string` 上命中 → 忽略 | `[]` | `ManisPocket/Clipboard.swift:409-423` |
| 暂停 | `ignoreEvents == true` → 全部忽略；`ignoreOnlyNextEvent` → 只忽略一次 | false / false | `ManisPocket/Clipboard.swift:184-194` |

UI 表现补充：
- 暂停时状态栏图标呈 disabled 外观（`ManisPocket/AppDelegate.swift:20-22,84-93`）。
- 自身写入剪贴板的内容会被 `fromManisPocket` 类型标记从而不再入库（`ManisPocket/Clipboard.swift:176-178,114`）。

### 9.5 更新检查（Sparkle）

- `SoftwareUpdater` 包装 `SPUStandardUpdaterController(startingUpdater: true)`（`ManisPocket/SoftwareUpdater.swift:19-24`）。
- 默认 feed = Info.plist 的 `SUFeedURL`；`betaUpdates == true` 时改用 `https://raw.githubusercontent.com/kaigedong/Manis-Pocket/feeds/appcast-beta.xml`（`ManisPocket/SoftwareUpdater.swift:43-49`、`ManisPocket/Info.plist`）。
- UI：General 页 3 个控件（自动检查 / 立即检查 / 包含 beta）。检查结果由 Sparkle 自己的系统弹窗展示，App 不自绘进度 UI。
- 移植风险：Sparkle 是 macOS-only 的 XPC + 签名校验体系，Linux 端必须换方案（自研 sparkle-less 更新器 / flatpak / AppImage updater）。GPUI 重写若保留 Rust 侧统一 UI，更新器仍必须平台分支。

### 9.6 粘贴栈（Paste Stack，多选后用 ⌘V 依次粘贴）

- 仅在 `multiSelectionEnabled == true` 时可用（当前硬编码 false，`ManisPocket/Observables/AppState.swift:12`）。
- 触发：多选状态下回车 → `startPasteStack`（`ManisPocket/Observables/History.swift:465-498`）。
- 交互：全局监听 ⌘V 的 keyDown/keyUp，keyUp 时 `handlePasteStack()` 消费下一条并写剪贴板（`ManisPocket/PasteStack.swift:8-30`）。
- 视觉（`ManisPocket/Views/PasteStackView.swift`）：
  - 最多折叠显示 **3** 层（`maxItems = min(3, items.count)`，`:75`）。
  - 第 0 层 offset 0；第 n 层 offset `-height * 0.75`；超出 maxItems 的层 offset `-height`（完全遮住）（`:11-19`）。
  - 缩放 `pow(0.98, index)`；不透明度 `pow(0.95, index)`；超限层 opacity 0（`:21-36`）。
  - 阴影：`color: sRGBLinear白 0 @ 0.1`，`radius: 2`，`y: 2`（open 时为 0）（`:93-97`）。
  - 圆角同 `Popup.cornerRadius`。
  - 序号徽标显示最底层项的索引（`indexTagFor`，`:64-72`）。
- 预览面板中的贴栈：`PasteStackPreviewView`，`padding = 2`，项间 `Divider().padding(.vertical, 2)`，容器 `.padding(.vertical, 2)`，背景 `.fill.tertiary`，圆角 `Popup.cornerRadius`（`ManisPocket/Views/PasteStackPreviewView.swift:17-44`）。

### 9.7 通知与声音

- 新增条目：`Notifier.notify(body: title, sound: .write)`（`ManisPocket/Observables/History.swift:253-255`）。
- 复制条目：`Notifier.notify(body: item.title, sound: .knock)`（`ManisPocket/Clipboard.swift:121-124`）。
- 走 `UNUserNotificationCenter`，需要用户授权；通知正文仅在 `alertSetting == .enabled` 时设置（`ManisPocket/Notifier.swift:9-42`）。

### 9.8 其他 UI 状态

| 状态 | 表现 | 来源 |
|---|---|---|
| 空列表 | 只有一个空的 ScrollView + header + footer；高度收缩到最小值（`headerHeight + 5`） | `ManisPocket/Observables/Popup.swift:100` |
| 搜索无结果 | 列表为空，footer 仍可键盘导航 | `ManisPocket/Observables/NavigationManager.swift:233-259` |
| 新增条目（`+` 按钮 / `history.addNew`） | 立即插入一行空文本并进入编辑态，选中它 | `ManisPocket/Observables/History.swift:436-463` |
| 编辑空文本后保存 | 该行被删除 | `ManisPocket/Observables/History.swift:411-415` |
| 取消编辑且原本就空 | 该行被删除 | `ManisPocket/Observables/History.swift:425-432` |
| 选中"关于" | 系统标准 About 面板，credits 含 GitHub 链接 + "Original work © Alex Rodionov" | `ManisPocket/About.swift:4-25` |
| 远处同步文件项 | 行内额外出现下载按钮 `icloud.and.arrow.down`；右键多一项 `Download <fileName>` | `ManisPocket/Views/HistoryItemView.swift:69-80,166-175` |

---

## 10. GPUI 移植难点

> 本节按"依赖的 AppKit 能力 → GPUI 0.2.x 现状 → 替代/降级方案"组织。
> 标 **⚠️ 需实测** 的条目表示 GPUI 0.2.2 的具体 API 名称/行为需在源码或 PoC 中确认，不要直接照抄本文的 API 名。

### 10.1 高风险（做不成就要改产品形态）

#### R1. 非激活浮动面板 + 失焦自动关闭 + 5 种定位

**依赖**：`NSPanel` + `styleMask [.nonactivatingPanel, ...]` + `level = .statusBar` + `override resignKey()` 自动关闭 + `PopupPosition` 5 种几何计算 + `NSStatusItem.button` 的屏幕矩形（`ManisPocket/FloatingPanel.swift:25-64,192-199`、`ManisPocket/PopupPosition.swift:32-70`）。

**GPUI 0.2.x 现状**：
- `WindowOptions` 提供 `window_background`、`titlebar: TitlebarOptions { appears_transparent, traffic_light_position, title }`、`kind: WindowKind`、`is_movable_by_window_background`（⚠️ 需实测字段名与 `WindowKind::PopUp` 的实际 NSWindowClass）。
- **没有**"不抢焦点"（nonactivating）语义、没有窗口 level 控制、没有 `resignKey` 回调（只有 `on_window_should_close` / `observe_window_activation`，激活观察 ≠ key 状态）。

**方案**：
1. 用 `objc2` / `objc2-app-kit` 在窗口创建后拿到 `NSWindow` 指针（GPUI macOS 后端可经 `window.window_handle()` → `raw_window_handle::RawWindowHandle::AppKit` 拿到 `NSView`，再 `view.window()`），**运行期改窗口属性**：`setLevel(NSStatusWindowLevel)`、`setCollectionBehavior`、`setHidesOnDeactivate(false)`、`setStyleMask`、`setOpaque(false)`、`setBackgroundColor(clear)`、`layer.cornerRadius`。
2. `nonactivatingPanel` 必须在 `NSPanel` 子类初始化时设置 —— 运行期改不了。可行路径：(a) 让 GPUI 用 `WindowKind::PopUp`（若它内部就是 borderless 面板）；(b) fork/patch GPUI 的 macOS 平台层让窗口类可注入；(c) 用 `NSPanel` 承载一个 GPUI 的 `NSView`（GPUI 支持把渲染视图挂到任意 NSView？⚠️ 需确认 `gpui::Application` 是否允许外部 NSView 作为渲染目标）。
3. 失焦关闭：轮询/监听 `NSWindowDidResignKeyNotification`（`NotificationCenter`），而不是依赖 GPUI 的事件。
4. 定位：`cx.displays()` 拿显示器 bounds（⚠️ 需实测），`NSWorkspace.frontmostApplication.windowFrame`（`ManisPocket/Extensions/NSRunningApplication+WindowFrame.swift`）与鼠标位置只能用 AppKit；`NSStatusItem` 按钮矩形同理。
5. 圆角 + 透明 + 阴影：`WindowBackgroundAppearance::Blurred` 只能给系统 vibrancy，不能指定 material `.popover` 与 blending `.behindWindow`。若要 1:1，需在 NSView 层级里插一个 `NSVisualEffectView`，或自己画背景 + 圆角裁剪（会丢失"透出桌面"的效果）。

**结论**：这一条决定整体架构 —— 建议先写一个 **200 行的 macOS PoC**，验证"GPUI 渲染 + NSPanel + 不抢焦点 + 失焦关闭"四件事同时成立，再决定是否继续。

#### R2. 文本输入 / IME / 焦点（中文输入是本项目主场景）

**依赖**：
- 面板打开即聚焦搜索框（`ManisPocket/Views/ContentView.swift:42-44`）；
- 有输入法候选窗时**必须忽略**所有按键语义（`ManisPocket/Views/KeyHandlingView.swift:20-28`）；
- 搜索框里按 ↑↓ 应移动列表选中（不是移动光标）；
- 行内编辑态是真正的多行 `TextField`（1...5 行）（`ManisPocket/Views/HistoryItemView.swift:188-200`）；
- 设置页有多处 `TextField` 与快捷键录制控件。

**GPUI 0.2.x 现状**：`gpui` 核心**不提供**成品文本输入控件（Zed 里的 `TextInput`/`Editor` 在 `ui`/`editor` crate 中，依赖 Zed 的 model 层）。官方只暴露 `EntityInputHandler` trait（IME 关键）与 `InputHandler`（键盘）。`gpui::input` 模块在 0.2.x 是否有可直接用的组件 ⚠️ 需实测。

**方案**：
1. 自己实现一个 `TextInput` 视图：内部 `Rope`/`String` + 光标 + 选中区，实现 `EntityInputHandler`（`text_for_range`、`selected_text_range`、`replace_text_in_range`、`marked_text_range` …）以接入 IME；
2. 或者直接以 `EntityInputHandler` + 自绘光标/mini 文本布局起步，只支持单行（搜索框）+ 多行（编辑态）两种；
3. 键盘优先级必须显式建模：`KeyBinding` 的 `context` 做优先级（例如 `"Popup && !SearchInput"` 上下文里方向键给列表，`"SearchInput"` 上下文里给光标），并在候选窗打开时把整个 `Popup` 上下文降级为不匹配任何 action。

**结论**：工作量约等于"在 GPUI 里重写一个 NSTextField"。这是第二大风险。

#### R3. 富文本行渲染（SF Symbols + 分段样式 + 中间截断）

**依赖**：
- 每行右侧的快捷键提示用符号 `⌃⌥⇧⌘` + 字符（`ManisPocket/Views/KeyboardShortcutView.swift`）；
- 高亮匹配要按 range 施加 bold/italic/underline/背景色（`ManisPocket/Observables/HistoryItemDecorator.swift:215-225`）；
- 标题单行 **中间截断** `truncationMode(.middle)`（`ManisPocket/Views/ListItemTitleView.swift:12,17`）；
- 大量 SF Symbols 图标：`doc.on.doc`、`trash`、`icloud.and.arrow.down`、`magnifyingglass`、`xmark.circle.fill`、`sidebar.left/right`、`plus`、`checkmark.circle.fill`、`photo.badge.exclamationmark`、`arrow.clockwise`、`pencil`、`stop`、`arrow.uturn.backward.circle.fill`、`questionmark.app.dashed`、设置页 7 个（`gearshape`/`externaldrive`/`arrow.triangle.2.circlepath`/`paintpalette`/`pin.circle`/`nosign`/`gearshape.2`）。

**GPUI 现状**：
- `StyledText` + `TextRun { len, style: TextStyle }` 支持 `font_weight`、`font_style`（italic）、`color`、`background_color`、`underline`（⚠️ 字段名需实测）。够表达 4 种高亮样式。
- **无** 中间截断：GPUI 只有 `truncate()`（尾部）与 `line_clamp`。
- **无** SF Symbols：没有 `NSImage(systemSymbolName:)`。

**方案**：
1. **中间截断自实现**：先测量全文宽度（`window.text_system().shape_line` 或 `TextLayout`），若超过可用宽度，则从尾部按字符二分去掉，直到 `前缀 + "…" + 后缀` 的宽度合适。需要缓存避免每帧测量。
2. **SF Symbols**：两条路
   - (a) 启动时用 `objc2-app-kit` 把需要的 symbol 渲染成 `RGBA` bitmap，喂给 GPUI 的 `Image`/`RenderImage`（一次性，图标集合固定，最省事，能保持系统观感）；
   - (b) 从 SF Symbols.app 导出 SVG/PNG 打包进 `assets/`，用 `svg()` 渲染（跨平台统一，但 Linux 端图标风格会变）。
   - 推荐 (a) macOS + (b) Linux 的双资源策略，抽象成一个 `icon(name) -> IconSource`。
3. **应用图标**：`NSWorkspace.icon(forFile:)` 取到 `NSImage` 后再转 `RenderImage`（`ManisPocket/ApplicationImage.swift:39-78`），Linux 端用 `.desktop` 文件 + icon theme 查找（GTK 侧已有实现可参考 `crates/manis-pocket-linux/`）。

### 10.2 中风险（有替代但要降级）

#### R4. 毛玻璃 / 液态玻璃背景

- `NSVisualEffectView(material: .popover, blendingMode: .behindWindow)`（`ManisPocket/Views/VisualEffectView.swift:6-7`）；macOS 26 用 `NSGlassEffectView(style: .regular)`（`:23`）。
- GPUI 的 `WindowBackgroundAppearance::Blurred` 是系统级 vibrancy，无法选 material/ blending，也可能与"窗口透明 + 自绘圆角"冲突。
- **降级**：半透明纯色底（例如 `rgba(30,30,30,0.72)` 深色 / `rgba(250,250,250,0.78)` 浅色）+ 1px 内描边模拟玻璃边缘；或按 R1 的方案插入真正的 `NSVisualEffectView`。
- macOS 26 的 Liquid Glass **只能放弃**（GPUI 无 API，且 Apple 未公开可用 API）。

#### R5. 设置窗口（7 个 pane + 原生控件）

- 现有实现依赖 `Settings` SPM 包：顶部工具栏 tab + 自动尺寸 + autosave frame（§1.5）。
- GPUI 里 **所有控件都要自绘**：Toggle（开关）、Picker（下拉菜单，GPUI 有 `popup_menu`/`dropdown` 但需自建）、Stepper（+/-）、Table（Pins 页，含可编辑单元格与多选）、TabView（Ignore 页 3 个 tab）、`fileImporter`（选 .app）、`sheet`（两个弹窗）、`KeyboardShortcuts.Recorder`（录制组合键）。
- **建议**：
  - Toggle/Picker/Stepper 做成一个 `gpui-component` 风格的小组件库（约 5 个组件，收益最大）；
  - Pins 的 Table 降级为"列表 + 行内编辑"（GPUI 的虚拟列表 + 自绘表头），保留三列但放弃列宽拖拽与系统多选；
  - Ignore 的 TabView 用自绘 segmented control；
  - 文件选择用 `rfd` crate（跨平台，macOS 上就是 NSOpenPanel 封装）；
  - sheet 用 GPUI 的二级窗口（`WindowKind::PopUp`）或页面内浮层。

#### R6. 快捷键录制与全局热键

- 全局热键：`KeyboardShortcuts`（Carbon `RegisterEventHotKey`）→ Rust 侧用 `global-hotkey` crate（macOS 上就是 Carbon，Linux 上 X11 grab；**Wayland 无全局热键**，需 xdg-desktop-portal `GlobalShortcuts`，且 GNOME 支持有限）。
- 录制控件：需要在 GPUI 里捕获"裸修饰键 + 任意键"并转成可序列化组合；`KeyChord` 里还有"当前布局下 ⌘ 切 QWERTY"的处理（`ManisPocket/KeyChord.swift:76-80`），Rust 侧可简化（用 `keyboard-types` crate 的 physical key + logical key 双轨）。
- **注意**：`pin`/`delete` 是"只在面板内生效"的伪全局快捷键（§7.2），Rust 侧不要注册成真全局热键。

#### R7. 剪贴板捕获与粘贴注入

- 捕获：`NSPasteboard.general` + `changeCount` 轮询 0.5s（`ManisPocket/Clipboard.swift:62-69,165-283`）。Rust 侧 `arboard` 在 macOS 上就是 NSPasteboard 封装，但**读不到 `fromManisPocket` 这类私有类型**；需要 `objc2-app-kit` 直接操作 `NSPasteboard`（或保留一小段 Swift/ObjC shim）。
- 粘贴注入：`CGEvent` 模拟 ⌘V（`ManisPocket/Clipboard.swift:127-147`）—— macOS 上必须用 `CGEventPost`，且需要辅助功能权限；Linux 上需要 `ydotool`/`libei`（Wayland 下 XTest 不可用）。
- **权限 UI**：源码里 `Accessibility.check()` 是空实现（`ManisPocket/Accessibility.swift:8-12`），但产品上必须提示用户授权，否则"复制并粘贴"静默失效。

#### R8. 系统集成项

| 能力 | 现状 | GPUI 方案 |
|---|---|---|
| 状态栏图标 | `NSStatusItem`（可变长度 + 高亮 + disabled 外观 + 标题文本） | `tray-icon` crate（macOS 上也是 NSStatusItem）+ objc2 补 `appearsDisabled`/`isHighlighted` |
| About 面板 | `NSApp.orderFrontStandardAboutPanel` | 自绘一个 GPUI 窗口（内容：App 名/版本/GitHub 链接/"Original work © Alex Rodionov"） |
| 通知 | `UNUserNotificationCenter` + `NSSound` | `notify-rust`（macOS 通过 osascript 效果差）或 objc2-UserNotifications；声音用 `rodio`/`NSBeep` |
| 更新 | Sparkle | 平台分支：macOS 用 `sparkle` 的 Rust 绑定或保留 Swift helper；Linux 自研 |
| OCR 标题 | Vision `VNRecognizeTextRequest(.fast)` | `ocrs`/`tesseract`（Linux 与 macOS 统一），或 macOS 继续调 Vision |
| 开机自启 | LaunchAtLogin | `auto-launch` crate（macOS: SMAppService / LaunchAgents；Linux: .desktop） |
| 屏幕/窗口信息 | `NSScreen`、`NSWorkspace.frontmostApplication.windowFrame` | objc2 直接调（无跨平台抽象） |

### 10.3 低风险（纯数据/逻辑，可直接搬）

- 搜索（exact/fuzzy/regexp/mixed）：Rust 侧已有 `crates/manis-pocket-core/src/search.rs`，UI 只需把结果 `ranges` 转成 `TextRun`。
- 排序（3 种 + 固定项置顶/置底）：`crates/manis-pocket-core/src/sort.rs`。
- 去重/容量限制：core 的 `add(maxSize, isUnlimited)`。
- 图片编码/缩放：`image` crate。
- 设置持久化：`serde` + 平台配置目录（macOS `~/Library/Application Support/ManisPocket`，与现有 DB 路径一致，便于迁移，见 `ManisPocket/Observables/History.swift:157-178`）。

### 10.4 建议的落地顺序（降低返工）

1. **PoC-A（1 天）**：GPUI 窗口 + `NSPanel` 注入 + 不抢焦点 + 失焦关闭 + 圆角/透明；只画一个空壳。**不过就换方案（例如保留 Swift 外壳 + GPUI 只做列表内容）。**
2. **PoC-B（1 天）**：单行 TextInput + IME + 方向键优先级。**不过就要评估是否用 `gpui::input`/第三方组件。**
3. 列表（虚拟化 `list()`）+ 行模板 + 4 种高亮 + 中间截断 + 图标管线。
4. Header / Footer / FilterBar（纯布局，直接照 §2–§5 数值写）。
5. 设置 7 页（先做 General + Appearance 验证 Toggle/Picker，再补其余）。
6. 预览侧栏（含开合动画与 5 种位置）。
7. 平台层收尾：状态栏、通知、更新、OCR、粘贴注入。

---

## 附录 A：所有 `Defaults` 键与默认值

来源：`ManisPocket/Extensions/Defaults.Keys+Names.swift:13-70`。

| 键（UserDefaults key） | Swift 键名 | 类型 | 默认值 | 对应 UI |
|---|---|---|---|---|
| `clearOnQuit` | clearOnQuit | Bool | false | Advanced 页 |
| `clearSystemClipboard` | clearSystemClipboard | Bool | false | Advanced 页 |
| `clipboardCheckInterval` | clipboardCheckInterval | Double | 0.5 | 无 UI（内部） |
| `appendModeEnabled` | appendModeEnabled | Bool | false | General 页 |
| `appendModeTimeWindow` | appendModeTimeWindow | Double | 1.0 | General 页 |
| `enabledPasteboardTypes` | enabledPasteboardTypes | Set<String> | `fileURL,heic,jpeg,png,tiff,html,rtf,string` 共 8 项 | Storage 页 3 开关 |
| `highlightMatch` | highlightMatch | enum | `.bold` | Appearance 页 |
| `ignoreAllAppsExceptListed` | ignoreAllAppsExceptListed | Bool | false | Ignore/Applications |
| `ignoreEvents` | ignoreEvents | Bool | false | Advanced 页 / ⌥点图标 |
| `ignoreOnlyNextEvent` | ignoreOnlyNextEvent | Bool | false | ⌥⇧点图标 |
| `ignoreRegexp` | ignoreRegexp | [String] | [] | Ignore/Regexp |
| `ignoredApps` | ignoredApps | [String] | [] | Ignore/Applications |
| `ignoredPasteboardTypes` | ignoredPasteboardTypes | Set<String> | 5 项（见 §6.5.2） | Ignore/Pasteboard |
| `imageMaxHeight` | imageMaxHeight | Int | 40 | Appearance 页 |
| `lastReviewRequestedAt` | — | Date | now | 无 UI |
| `menuIcon` | menuIcon | enum | `.manispocket` | Appearance 页 |
| `migrations` | migrations | [String: Bool] | [:] | 无 UI |
| `numberOfUsages` | — | Int | 0 | 无 UI |
| `pasteByDefault` | pasteByDefault | Bool | false | General 页 |
| `pinTo` | pinTo | enum | `.top` | Appearance 页 |
| `popupPosition` | popupPosition | enum | `.cursor` | Appearance 页 |
| `popupScreen` | popupScreen | Int | 0（主屏） | Appearance 页（多屏时） |
| `previewDelay` | previewDelay | Int | **500** | Appearance 页 |
| `removeFormattingByDefault` | removeFormattingByDefault | Bool | false | General 页 |
| `searchMode` | searchMode | enum | `.exact` | General 页 |
| `showFooter` | showFooter | Bool | true | Appearance 页 |
| `showInStatusBar` | showInStatusBar | Bool | true | Appearance 页 |
| `showRecentCopyInMenuBar` | showRecentCopyInMenuBar | Bool | false | Appearance 页 |
| `showSearch` | showSearch | Bool | true | Appearance 页 |
| `searchVisibility` | searchVisibility | enum | `.always` | Appearance 页 |
| `showSpecialSymbols` | showSpecialSymbols | Bool | true | Appearance 页 |
| `showTitle` | showTitle | Bool | true | Appearance 页 |
| `historySize` | size | Int | **500** | Storage 页 |
| `isUnlimitedHistory` | isUnlimitedHistory | Bool | false | Storage 页 |
| `sortBy` | sortBy | enum | `.lastCopiedAt` | Storage 页 |
| `suppressClearAlert` | suppressClearAlert | Bool | false | 清空确认框内 |
| `windowSize` | windowSize | NSSize | **450 × 800** | 无直接 UI（拖动改变） |
| `windowPosition` | windowPosition | NSPoint | (0.5, 0.8) | Appearance 页重置按钮 |
| `showApplicationIcons` | showApplicationIcons | Bool | true | Appearance 页 |
| `previewWidth` | previewWidth | CGFloat | 400 | 无直接 UI（拖动分隔条） |
| `betaUpdates` | betaUpdates | Bool | false | General 页 |
| `syncEnabled` | syncEnabled | Bool | false | Sync 页 |
| `syncDeviceName` | syncDeviceName | String | `Host.current().localizedName ?? "Mac"` | Sync 页 |
| `syncDeviceID` | syncDeviceID | String | 随机 UUID | Sync 页（内部） |
| `syncDiscoverable` | syncDiscoverable | Bool | false | Sync 页 |

## 附录 B：GPUI 侧需要保留的"行为不变量"（验收清单）

移植完成后逐条核对，全部通过才算 1:1：

1. 打开面板后光标已在搜索框内，且可以直接输入中文（候选窗出现时不误触方向键语义）。
2. `Esc` 关闭面板；`↩` 复制并关闭；`⌘,` 打开设置；`⌃Space` 开关预览；`⌥P` 固定；`⌥⌫` 删除。
3. 面板失焦（点桌面/其它 App）立即关闭；打开系统确认框时不关闭。
4. 按住 ⇧⌘C 不放、连按 C 键可在列表中循环，松开 ⇧⌘ 时复制当前项并关闭。
5. 面板高度 = 内容高度，且被 `windowSize.height` 上限截断；内容变高时窗口**底边固定、向上生长**。
6. 列表第一屏 9 行右侧分别显示 `⌘1`…`⌘9`；按住 ⌥ 时变成 `⌥1`…`⌥9`；按住 ⌥⇧ 时变成 `⌥⇧1`…`⌥⇧9`。
7. 固定项显示其 pin 字母的三套组合；顶部/底部固定区按 `pinTo` 决定。
8. 搜索命中按 `highlightMatch` 渲染（默认加粗）；标题中间截断。
9. 标题行 hover 时右侧浮现"复制/删除"两个 24×22 按钮，行文字右缩 56；点复制出现 1.5s 绿色 "Copied!" 气泡。
10. Footer 5 项文案与快捷键正确；按住 ⇧ 时第一项从"清除"变"全部清除"。
11. 筛选栏图标 18×18，选中/变暗/红框三种状态参数与 §5 一致；点击可切换筛选。
12. 图片项行高随缩略图（≤40 高）增长；标题为十六进制颜色时显示 12×12 色块。
13. 7 个设置页的控件顺序、文案、默认值与本文件 §6 完全一致（含 Sync 的配对 PIN 方块 32×40）。
14. 预览侧栏：默认宽 400、最小 200；右扩不下时向左扩；打开/关闭 0.25s ease-in-out。
15. 复制行为矩阵（§7.5）在 4 种 `pasteByDefault`/`removeFormattingByDefault` 组合下全部正确。
