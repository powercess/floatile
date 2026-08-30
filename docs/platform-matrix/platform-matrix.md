# 三端能力矩阵

> 状态：Proposed
> 性质：能力预测，不是实测结果。P0 结束必须回填第 5 节。
> 语义：`预期支持` · `条件支持` · `预期不支持`。任何结论都以带环境的实测记录为准。

## 1. 总览矩阵

| 能力 | Windows 10/11 | macOS 12+ | Linux X11 | Linux Wayland |
|------|---------------|-----------|-----------|---------------|
| 无边框窗口 | 预期支持 | 预期支持 | 预期支持 | 预期支持 |
| 透明/半透明窗口 | 预期支持：DWM / `WS_EX_LAYERED` | 预期支持：NSWindow 非不透明 | 条件支持：依赖合成器 | 预期支持：ARGB surface |
| Always-on-top | 预期支持：`HWND_TOPMOST` | 预期支持：floating level | 预期支持：`_NET_WM_STATE_ABOVE` | 条件支持：layer-shell/合成器 |
| 点击穿透 | 预期支持：window hit-test | 预期支持：`ignoresMouseEvents` | 预期支持：XShape input region | 预期不支持：核心协议无统一能力 |
| 桌面附着 | 条件支持：WorkerW hack，脆弱 | 条件支持：授权与行为风险 | 预期支持：BELOW/DOCK | 条件支持：layer-shell background |
| 多显示器 | 预期支持 | 预期支持 | 预期支持 | 预期支持 |
| DPI 缩放 | 预期支持：per-monitor v2 | 预期支持：`backingScaleFactor` | 条件支持：无统一每屏 DPI | 预期支持：合成器缩放 |
| 显示器热插拔恢复 | 预期支持：`WM_DISPLAYCHANGE` | 预期支持：`NSScreen` 通知 | 条件支持：RandR/桌面环境差异 | 预期支持：output 事件 |
| 快捷键注册 | 预期支持 | 预期支持 | 条件支持：依赖桌面环境 | 条件支持：依赖全局快捷键协议 |
| 透明 + 点击穿透同时可用 | 预期支持 | 预期支持 | 预期支持 | 预期不支持 |

## 2. 关键能力详解

### 2.1 点击穿透（Click-through）

| 平台 | 机制 | 备注 |
|------|------|------|
| Windows | 设置 `WS_EX_TRANSPARENT`（不参与 hit-test）+ `WS_EX_LAYERED`（每像素 Alpha）；交互时清除该样式 | 窗口整体生效，无法按像素区域 |
| macOS | `window.setIgnoresMouseEvents(true)` | 同上，整体生效 |
| X11 | XShape 将输入区域设置为空 shape；`_NET_WM_WINDOW_TYPE_DOCK` 辅助 | 整体生效；无合成器时仍可穿透（输入区域独立于渲染） |
| Wayland | **核心协议不提供**。无标准机制让普通窗口忽略输入。可选：layer-shell 提供部分（`exclusive_zone` 只影响布局抢占，不解决点击穿透） | **P0 最大风险，必须降级** |

**Wayland 降级策略**（P0 必须实现并实测）：
1. 检测合成器能力（是否支持 `wlr-layer-shell`、`zwlr_foreign_toplevel` 等）。
2. 支持 layer-shell → 用 layer-shell 实现置顶与桌面附着，但点击穿透仍受限 → 降级为「编辑模式常驻边框 + 显式最小化控件」。
3. 不支持 → 走 XWayland（通过 `GDK_BACKEND`/环境检测），在 XWayland 内可正常点击穿透，但存在缩放/合成差异。
4. 最坏情况：纯 Wayland 且无 layer-shell → 禁用点击穿透，始终可交互，提供「收起/悬浮态」替代，并明确标记降级原因。

### 2.2 透明窗口

| 平台 | 机制 | 注意 |
|------|------|------|
| Windows | `WS_EX_LAYERED` + per-pixel alpha，或 DWM 下的 `SetLayeredWindowAttributes`；Slint 侧设置透明背景 | 需要 DWM 合成 |
| macOS | `NSWindow.isOpaque = false`，背景透明；`NSWindow.CollectionBehavior` 控制 Spaces | 全屏切换 Spaces 时需处理 |
| X11 | 需要合成器（compositor），否则透明显示为黑色 | 无合成器时降级为不透明背景 |
| Wayland | ARGB 表面 + xdg-shell；合成器负责混合 | 由合成器决定 |

### 2.3 Always-on-top 与桌面附着

- Windows：`HWND_TOPMOST`（置顶）。桌面附着用 WorkerW 是已知脆弱 hack，P0 标记为「可选实验」，不作为验收项。
- macOS：`NSWindow.Level = .floating/.statusBar`；桌面附着需要辅助功能授权，P0 不做。
- X11：`_NET_WM_STATE_ABOVE` / `_NET_WM_STATE_BELOW`。
- Wayland：`wlr-layer-shell` 的 `layer`（top/overlay/bottom/background）提供置顶与背景；非 wlroots 合成器（GNOME Mutter）不支持 → 走 XWayland 或普通窗口。

## 3. 与 Slint 的关系

- 透明与初始窗口标志通过 winit `WindowAttributes` 设置；Slint 的组件属性同步会重写窗口级别，
  因此 Always-on-top 还需在原生窗口创建后由 `floatile-platform` 再次应用。
- 无法在 winit 表达的（Windows click-through、macOS ignoresMouseEvents、X11 XShape）通过 `raw-window-handle` 获取原始窗口句柄后，在 `floatile-platform` 内用平台 API 直接调用，**不得外泄到业务层**。

## 4. 降级策略汇总

| 场景 | 降级行为 |
|------|---------|
| 无合成器（X11） | 背景不透明；置顶仍按 WM/EWMH 能力处理；记录 `CompositorNotDetected` |
| X11 无 SHAPE 或恢复热键注册失败 | 禁止启用点击穿透，Show 模式保留可交互窗口；记录扩展或热键错误 |
| X11 WM 不声明 `_NET_WM_STATE_ABOVE` | 置顶降级为普通窗口；记录 `WindowManagerUnsupported` |
| 纯 Wayland 无 layer-shell | 点击穿透禁用、置顶降级为普通窗口；提供手动置顶控件 |
| 无 GPU / 驱动异常 | Slint 软件渲染器回退；记录 GPU 信息到诊断日志 |
| 显示器热插拔找不到原屏 | 运行态落回主屏并标记 `lost_monitor`；持久化保留原 monitor key 与 monitor-local 矩形，原屏重新接入后恢复，不丢数据 |
| 高 DPI 混合 | 以窗口所在屏的 scale factor 为准，跨屏拖动时重算逻辑尺寸 |

## 5. P0 实测回填栏

> 每个平台跑一遍：透明 → 置顶 → 点击穿透 → 编辑模式 → 拖拽 → 多屏 → 热插拔 → 热插拔后布局恢复。
> 记录：实测结果、环境（WM/合成器/版本）、截图、性能。

| 日期 | 平台/环境 | 透明 | 置顶 | 穿透 | 编辑模式 | 多屏 | 热插拔 | 备注 |
|------|-----------|------|------|------|----------|------|--------|------|
| 2026-08-13 | Windows 11，DWM 合成桌面，GPU 由 Slint 默认后端（dev 构建，commit 待定） | ✅ 无边框实测（`WS_POPUP`，`CAPTION/BORDER/SYSMENU` 清除）；圆角外角落像素 Alpha=0 透明生效 | ✅ 探测返回 `always_on_top=true`，实测 ex-style 含 `WS_EX_TOPMOST`，窗口盖过普通窗口 | ✅ Edit 模式 `TRANSPARENT=false`、Show 模式 `TRANSPARENT=true`（+LAYERED）；模式切换与全局热键 Ctrl+Shift+E 实测联动 | ✅ 编辑控件（边框/设置/展示/删除/缩放手柄）显示，Show 模式隐藏；拖拽（WM 拖动）与缩放（手柄 274x158→442x222）实测 | 未测 | 未测 | 探测日志：`kind=Windows click_through=true always_on_top=true`；窗口进程存活无崩溃；winit 0.30 顶层窗口 `with_decorations(false)` 不生效，由 `floatile-platform` 创建后强制移除（已记录）；穿透的视觉 Alpha 混合需在真实使用场景复核 |
| 2026-08-16 | Arch Linux，Xvfb 21.1.24（1280×720）、Openbox 3.6.1、picom 13/xrender；Mesa llvmpipe 26.1.7（非加速）；VMware 4 vCPU；release，commit `99b8781` | ✅ picom 的 `_NET_WM_CM_S0` owner 被 `x11rb` 探测；无标题栏，洋红背景从圆角外与 0.92 Alpha 内容透出。停止 picom 后探测为 `compositing=false`，背景降级不透明且宿主存活 | ✅ `_NET_WM_STATE_ABOVE`；raise 普通全屏 xmessage 后 Floatile 仍在上层 | 未实现：X11 `click_through=false`，未声称通过 | 编辑控件可见；拖拽 `(0,0)→(200,100)`，尺寸保持 `260×120`；缩放与 Show 模式未测 | 未测 | 未测 | 首帧 `110.10 ms`；稳态 CPU `0.15–0.18%` 单核；交互前 RSS `218.9 MiB`；按需渲染约 `1.98 FPS`。截图：测试机 `~/floatile-test/logs/final-aot-transparent.png`、`final-no-compositor.png`。这是 headless Xvfb 证据，不替代物理 X11、Wayland 或 GPU 加速环境实测 |
| 2026-08-16 | Arch Linux，Xvfb 21.1.24（SHAPE、RandR）、Openbox 3.6.1、picom 13/xrender；release，`agent/s2-linux-platform`（base `99b8781` + 未提交变更） | ✅ 合成器开启时仍透明；停止 picom 后 `compositing=false reason=CompositorNotDetected`，不影响 XShape 能力 | ✅ Openbox 声明 `_NET_WM_STATE_ABOVE` 时探测为可用；停止 WM 后 `always_on_top=false reason=WindowManagerUnsupported` | ✅ Edit 模式同坐标点击由宿主“设置”控件拦截，底层 xmessage 存活；Show 模式 `click_through=true` 后点击同一坐标命中底层 `HIT` 并使其 exit 0；Ctrl+Shift+E 恢复 Edit 后再次拦截 | ✅ Show 隐藏宿主控件；X11 passive grab 热键恢复 Edit，并以 XShape `None` 重置输入区 | 仅验证 RandR 单活动输出：`screen`、`1280×720@0,0`、primary；无 EDID 时 key 明确降级为 `x11-output-screen`；未验证真实多屏 | 未测 | 能力日志同时记录可用性与原因；穿透只有在 SHAPE 与恢复热键都成功时才启用。截图：测试机 `~/floatile-test/logs/s2-show-click-through.png`、`s2-edit-restored.png`。这是 headless 单输出 Xvfb 证据，不替代物理多屏 X11、DPI、热插拔或 Wayland 实测 |
| 2026-08-16 | Arch Linux，headless weston 14.0.2（docker/alpine 容器，`--backend=headless`，wayland-test socket）；VMware 无 GPU，Mesa libEGL fallback；dev 构建，`agent/s3-wayland-probe` | 🟡 探测 `compositing=true`（Wayland 协议本身由合成器提供）；headless 环境无法做像素级视觉混合验证（weston 未实现 wlr-screencopy，grim 截图失败） | 🟡 显式降级 `always_on_top=false reason=ProtocolUnsupported`，符合 §2.3/§4 预期 | 🟡 显式降级 `click_through=false reason=ProtocolUnsupported`，符合 §2.1 降级策略；宿主保持可交互 | ✅ winit 纯 Wayland 窗口创建成功（`floatile-shell running` 无连接错误）；首帧 `77.9 ms`；时钟按需渲染每秒 1 帧；编辑控件渲染正常 | 未测 | 未测 | 探测日志：`kind=Wayland compositing=true click_through=false(ProtocolUnsupported) always_on_top=false(ProtocolUnsupported)`；稳态 CPU `0.14–0.15%` 单核、RSS `120.7 MiB`（低于 Xvfb 同场景 218.9 MiB）；进程存活无崩溃。headless 无 portal/DBus，sctk_adwaita 主题与 a11y 报超时/警告（非致命）。这是容器合成器的协议层证据，验证 F3/R1 降级代码路径与 winit 纯 Wayland 链路；不替代 sway（wlroots）与 GNOME/Wayland 桌面会话实测 |
| 2026-08-16 | Arch Linux，Xorg 21.1.24 `:0`、Xfce 4.20/Xfwm4 内置合成、VMware `Virtual-1` 1280×800；release SHA-256 `e215c1df708a2a8cf98dc81c7d3ae0e3fd830a35090b4fcb3f3ca31e0d7a241f`，`agent/s2-linux-platform` 未提交变更 | ✅ `compositing=true`，桌面壁纸从圆角外与 0.92 Alpha 内容透出 | ✅ `always_on_top=true` 且运行日志确认 `always-on-top applied` | ✅ 在 Thunar 中双击裸二进制启动后，拖动两次并分别短按“展示”均得到 Show 穿透；覆盖区指针命中 Thunar 窗口，点击清除底层文件选中状态 | ✅ 修复 Slint 指针 grab 与 X11 原生拖动冲突：winit 事件过滤器在 Slint 处理按下前识别拖动区并 `PreventDefault`，顶部控件和缩放手柄继续传播；`unmap→map→activate` 自动重放 Edit；Ctrl+Shift+E 幂等恢复 | 仅验证单输出 `Virtual-1`、`1280×800@0,0`、primary；无 EDID 时 key 为 `x11-output-Virtual-1` | 未测 | 已按用户真实路径在 Thunar 双击 `/home/cn059/floatile-test/bin/floatile-shell` 验证，而非通过 SSH 启动。截图：测试机 `~/floatile-test/logs/doubleclick-drag-then-show-fixed.png`、`doubleclick-second-drag-show-clickthrough.png`、`doubleclick-drag-edit-restored.png`；Xfwm4 报告“禁用 resize 无效”，当前不影响 S2 手柄缩放路径 |
| 2026-08-17 | macOS 15.7.5、Apple M4（arm64）、Quartz 合成桌面、单屏 1920×1080；release SHA-256 `e9dbc133199fb7dac41efad9a1a31e4ef03da3db001efc1fa3eccea6aa6e4fc4`，`agent/macos-platform`（base `669612f`） | 🟡 机制已接入：probe `compositing=true`，winit `with_transparent(true)` → NSWindow 非不透明；Quartz 实测整窗 `alpha=1.0`（逐像素 Alpha 由 Slint 合成），视觉逐像素透明未在本环境复核 | ✅ Quartz `CGWindowListCopyWindowInfo` 实测 `layer=3`（`kCGFloatingWindowLevel`），日志 `always-on-top applied`，窗口盖过普通窗口层 | 🟡 机制已接入：probe `click_through=true`；Edit/Show 切换调用 `NSWindow.setIgnoresMouseEvents`；未交互实测穿透点击 | ✅ 无边框（窗口标题为空）、编辑控件与时钟渲染；布局保存 + 跨重启恢复 `layout restored lost_monitor=false`，显示器键 `macos-uuid-A8B23E1A-…`（`CGDisplayCreateUUIDFromDisplayID`）；拖拽/缩放机制已接入，未交互实测 | 仅验证单活动输出：NSScreen 枚举 `1920×1080@0,0`、primary、`677×381 mm`；未验证真实多屏 | 未测 | 探测日志：`kind=MacOS compositing=true click_through=true always_on_top=true`；Carbon `RegisterEventHotKey` 注册成功（`global hotkey registered (Ctrl+Shift+E)`，无需辅助功能授权）；首帧 `115.8 ms`、稳态 RSS `67.5 MiB`、空闲 CPU `0.0%`、按需渲染 `2.55 FPS`；winit 0.30 顶层窗口 `with_decorations(false)` 在 macOS 生效（无需平台层移除装饰）；进程存活无崩溃。截图：`/tmp/floatile-mac.png`。穿透/拖拽/缩放的交互实测待人工在真实桌面复核 |
| 2026-08-25 | Arch Linux 7.1.8、Xvfb 21.1.24（1280×720×24，无 WM/合成器）、Slint 默认后端、dev 测试构建，`agent/instance-control-surface`（`5e5e708`） | 未测（本项仅验证 PP-M1 动态实例生命周期） | 未测 | 未测 | ✅ 自动测试实例化并显示插件控制窗，创建同一 Installation 的两个真实 Wasmtime/Slint 窗口并观测二者进入 running；第二实例停止、安装内容临时缺失后单独 failed，第一实例持续 running；恢复安装并手动 retry 后双实例重新 running | 单 Xvfb 输出，未测 | 未测 | 命令：`RUSTC_WRAPPER= xvfb-run -a -s '-screen 0 1280x720x24' cargo test -p floatile-shell --test persistent_instance_lifecycle --locked -- --nocapture`；1 test passed，复验耗时 1.49 s。测试期间 SQLite/文件操作在协调 worker，Slint timer 仅消费 observed 快照和发送有界命令；无视觉截图，不替代真实桌面交互与跨平台证据 |
| 2026-08-26 | Arch Linux 7.1.8、Xvfb 21.1.24（1280×720×24，无 WM/合成器）、Slint 默认后端、dev 测试构建，`agent/pp-m4-rust-author-loop` | 未测（本项仅验证 PP-M4 作者预览/运行） | 未测 | 未测 | ✅ `floatile preview` 从项目构建、校验、临时安装后派生 shell preview-host，使用正式 renderer/Slint/Wasmtime/Broker；clock guest `start()` 后 observed lifecycle 进入 running。generation 替换测试先启动一代预览，再派生第二代、终止第一代，第二代独立进入 running。`run` 两次复用相同 digest 的安装，分别创建持久实例、推进 generation 并进入 running。仓库外干净目录从独立 SDK 包快照串行通过 `new/check/test/dev --once/preview/build/install/run/inspect` | 单 Xvfb 输出，未测 | 未测 | 命令：`RUSTC_WRAPPER= FLOATTILE_PREVIEW_HOST=$PWD/target/debug/floatile-preview-host xvfb-run -a -s '-screen 0 1280x720x24' cargo test -p floatile-cli --test sdk_package --locked clean_directory_completes_the_rust_author_loop -- --ignored --nocapture`；1 test passed，25.77 s。无截图，不替代真实桌面交互、Windows、macOS 或 Wayland 验证 |
| 2026-08-29 | Windows 11 x64、DWM 合成桌面、单活动显示器、dev 构建，`agent/windows-alpha-shell`（commit 待定） | 未复测 | 未复测 | 未复测 | ✅ 无边框 Widget 使用右下角宿主手柄放大/缩小，内容随窗口尺寸变化；托盘显式退出后进程归零，立即重启时位置、尺寸与模式从 SQLite 恢复；托盘、管理窗口和单实例继续正常 | ✅ `EnumDisplayMonitors` 自动实测识别活动主显示器；仅单屏，未验证跨屏 | 未测 | 自动测试 `windows_enumerates_an_active_primary_monitor` 通过；人工测试覆盖缩放最小/最大方向、托盘退出、数据库释放、连续启动单实例与布局恢复。Widget 内“删除”按产品语义移除布局记录，不应恢复；未验证真实多屏、混合 DPI 与热插拔。 |
| 2026-08-30 | Windows 11 x64、DWM、125% DPI、单活动显示器、dev 构建，`agent/windows-alpha-plugin-install`（commit 待定） | ✅ 第三方 `dev.floatile.system-monitor` 由显式宿主 `FloatileRuntimeWidget` 创建，无系统标题栏 | 未复测 | ✅ 运行实例启动时恢复热键以 `RegisterHotKey(NULL, ...)` 注册到 Slint UI 线程队列；点击“展示”后宿主框架消失，电脑控制发送 Ctrl+Shift+E 后编辑框架恢复 | ✅ 电脑控制实测运行时插件拖动、缩放及重启恢复；管理/展示/热键恢复；控制面层级、停止/启动和删除确认。stopped 后性能窗口消失；无可见窗口的仅托盘状态第二次启动 exe 后管理中心出现且进程仍为 1。运行实例冷启动仅枚举 `Floatile Resource Monitor`，窗口捕获从原先的 Clock+插件两张表面收敛为唯一 `360×310` 插件表面，证明插件模式不再创建隐藏 Clock HWND。原生文件选择器位于管理中心前景，取消恢复 busy；选择现有测试包后路径回填并走正式安装链，同版本返回稳定 `FINST_ALREADY_INSTALLED`，后续刷新仍显示，未产生重复实例。管理中心显示精确版本引用数；电脑控制选择被实例 #2 引用的 `0.1.0` 后显示阻塞原因且不提供卸载动作，并正确显示未签名开发信任、来源路径及 `system:cpu`、`system:memory`、`timer:schedule` 权限。并存安装机械生成的 `0.2.0` 后，运行态只提示可切换；停止实例后出现“切换到 0.2.0”，确认态解释配置与权限语义；确认后引用数从 `0.1.0` 原子转移到 `0.2.0`，重新启动的 Widget 正常持续采样。再次打开回滚确认态时明确显示“权限声明无变化”和目标配置校验提示，取消后绑定仍为 `0.2.0` 并恢复运行。Windows UI Automation 将安装/实例行识别为带组合名称的选项，将停止、保存、启动、切换和删除识别为按钮；Tab 焦点环可见，Enter 实测选择版本和停止实例成功并随后恢复启动 | 单屏枚举键 `windows-device-\\.\DISPLAY145`；未验证跨屏 | 未测 | Windows platform 16/16、Shell lib 63/63、Shell bin 2/2、renderer 18+5 通过；目标 crate Clippy `-D warnings` 通过。电脑控制覆盖布局、模式、层级、生命周期、唯一插件窗口、线程热键恢复、仅托盘唤醒、原生选择/取消/安装错误、删除取消、安装版本卸载门禁、信任/来源/权限展示、stopped 实例显式升级和键盘/无障碍语义；真实无引用版本的卸载确认仅由自动化测试覆盖，未通过 UI 执行删除。真实多屏、混合 DPI 跨屏和热插拔仍未验证。 |
| 2026-08-30 | Windows 11 x64、当前用户桌面、dev 构建，`agent/windows-alpha-plugin-install`（commit 待定） | 未复测 | 未复测 | 未复测 | ✅ AI Balance stopped 实例显示 Connection 缺失说明、遮罩 secret 表单和 disabled/ready 保存状态；电脑控制输入测试 provider/account/token 后成功创建并显示“已授权 1 个连接”。终止宿主并冷启动后授权仍存在；撤销先进入默认焦点为“取消”的确认态，确认后恢复零授权表单。测试 token 已从 UI 清空，扫描 `%APPDATA%\\floatile` 无明文，最后 grant 删除后 credentials 目录无文件 | 未测 | 未测 | Credential Manager 在测试执行上下文返回 Win32 1312，宿主自动使用 DPAPI machine-scope 密文 + 当前用户 AppData ACL；platform Credential Manager/DPAPI 2 tests、services 跨 vault 实例恢复、shell Connection 创建/撤销/SQLite 无 secret 测试通过。该 fallback 的本机管理员威胁需在 Beta 前复核。 |
| 2026-08-30 | Windows 11 x64、当前用户桌面、dev 构建，`agent/windows-alpha-plugin-install`（commit 待定） | 未复测 | 未复测 | 未复测 | ✅ 第二个持久 Connection 的 SQLite ID 为 2，而 AI guest 仍以实例局部 handle 1 成功创建统一 Widget；失败 HTTPS 探测经有界 supervisor 队列把真实 ID 2 更新为 `degraded`，管理中心只显示 provider、非秘密 account 与分类健康状态。运行实例不再显示更新/撤销危险动作，stopped 实例显示更新入口。电脑控制用虚构本机凭证执行轮换后，credential generation 从 0 增至 1、health 重置为 `unknown`，输入框立即清空；空输入的保存动作同时在视觉和 UI Automation 中标记 disabled。凭证编辑默认焦点位于“取消”，Escape 实测退出编辑且不修改 Connection。新旧测试字符串扫描 `%APPDATA%\\floatile` 均无明文 | 未测 | 未测 | Shell lib 70/70、目标 platform/services/shell Clippy `-D warnings`、AI `floatile check --deny-warnings` 通过；服务测试证明 guest 猜测真实 ID 被拒绝而 health listener 仍收到真实 ID。当前测试 Connection #2 的最终撤销/凭证清理等待桌面删除确认，不把待清理状态记为通过。 |
| 2026-08-30 | Windows 11 x64、当前用户桌面、dev 构建，`agent/windows-alpha-plugin-install`（commit 待定） | 未复测 | 未复测 | 未复测 | ✅ 管理中心概览显示宿主深色主题的“登录 Windows 后自动运行 Floatile”开关，Windows UI Automation 识别为 checkbox；从 AI 实例详情点击常驻“概览”可立即返回设置卡。当前注册表未启用，未替用户改变系统设置。两个实例临时设为 stopped 后，以 `--background` 启动只保留 1 个 `floatile-shell` 进程且无可枚举窗口；再次普通启动唤醒管理中心，进程仍为 1。验证后资源监控实例恢复 running | 未测 | 未测 | platform autostart 命令编码测试通过，Shell 70/70、platform/shell Clippy `-D warnings`、diff 检查通过。注册表实际 enable/disable 写入需要用户动作时确认，尚未执行；后台模式的真实 Windows 托盘宿主与单实例 activation 已验证。 |
| 2026-08-30 | Windows 11 x64、当前用户桌面、dev 构建，`agent/windows-alpha-plugin-install`（commit 待定） | 未复测 | 未复测 | 未复测 | ✅ 常驻“诊断”入口显示深色主题的只读脱敏摘要；电脑控制点击“复制摘要”后按钮短时显示“已复制”，无需手工选择文本。把系统剪贴板粘贴到安装路径框后可见摘要末尾 `connection=unknown` 且安装动作变为可用，证明剪贴板收到摘要；测试输入随后全选清空，路径恢复占位符且安装动作重新禁用 | 未测 | 未测 | 自动测试构造包含 private config、provider、account、secret 与 `cred://` 的快照，证明输出小于 8 KiB 且均未泄露；实例条目限制为 32。首次单帧 `select-all/copy` 的真实桌面测试复制为空，修复为焦点、全选、复制三阶段后复测通过。 |
