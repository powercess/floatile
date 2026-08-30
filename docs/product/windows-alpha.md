# Floatile Windows 内部 Alpha

> 状态：Accepted
>
> 范围：Windows 11 x64 内部可用性验证，不授权公开分发
>
> 基线：2026-08-29

本文定义 Windows-first 内部 Alpha 的增量产品要求。它不删除
[`requirements.md`](requirements.md) 的 P0 跨平台要求，也不解除 NFR-LEGAL-01 发布门。

## 1. 宿主壳增量要求

| ID | 要求 | 验收 |
|---|---|---|
| WA-HOST-01 | Windows 桌面宿主必须在当前用户会话中保持单实例；第二次启动不得创建第二个长期进程，并应唤醒现有管理中心。 | 同一 Windows 会话连续启动两次 `floatile-shell`，进程数保持为 1；即使所有 Widget 和管理窗口都已关闭、宿主仅在托盘运行，第二次启动也会显示并提升现有管理中心；首个实例继续响应。 |
| WA-HOST-02 | 宿主必须提供系统托盘入口；管理中心关闭后收起到托盘，托盘提供重新打开与显式退出。 | Windows 通知区显示 Floatile 图标；左键或菜单“打开 Floatile”显示插件与实例管理窗口；关闭该窗口后宿主与 Widget 继续运行；菜单“退出 Floatile”结束宿主。 |
| WA-HOST-03 | 管理中心与 Widget 必须使用不同窗口角色；Widget 不进入任务栏/Alt+Tab，管理中心按普通应用窗口呈现。 | Widget 保持无边框置顶但不出现在任务栏和 Alt+Tab；管理窗口显示 Windows 系统标题栏，可从任务栏和 Alt+Tab 切换。 |
| WA-HOST-04 | 显式退出必须有界停止插件、保存布局、释放平台资源并关闭数据库；单个插件不得无限阻塞退出。 | 托盘“退出 Floatile”先保存当前布局；宿主停止计时器、托盘、热键和插件监督器，参考插件 worker 最多等待 3 秒，超时后继续退出；再次启动后布局可恢复且数据库可重新打开。 |
| WA-INSTALL-01 | 管理中心必须允许用户通过原生文件选择器安装本地开发 `.floatile` 包；对话框、包读取、校验与原子安装不得阻塞 Slint 主线程，也不得绕过正式包预算和路径校验。 | 点击“浏览”打开位于管理中心前景的 Windows `*.floatile` 选择器；取消可恢复，选择后路径回填。合法包安装后自动出现在插件列表；缺失、超限、恶意或同版本已安装包显示稳定结果且不留下部分安装目录或重复实例。 |
| WA-INSTALL-02 | 管理中心必须展示全部并存安装版本，并只允许卸载没有任何精确实例引用的版本；卸载不得留下可被运行时加载的半删除目录。 | 每个版本显示引用实例数。被实例引用的版本不提供卸载动作并说明阻塞实例；未引用版本经二次确认后，先原子移出可加载命名空间再清理文件，其他版本和实例不受影响。 |
| WA-INSTALL-03 | 实例版本更新与回滚必须是 stopped 实例上的显式操作；目标 Installation、信任和 canonical config 必须在持久换绑前复核，运行实例不得静默升级。 | 并存版本存在时，运行态只提示目标而不提供切换动作；停止后显示目标版本并进入确认态，明确列出新增、移除和参数变化的 capability。确认后 SQLite 原子换绑并审计 upgrade/rollback，启动使用新版本；目标缺失、异插件、签名失效、配置不兼容或并发变化均保持原绑定。 |
| WA-WIDGET-01 | 每个第三方插件实例必须使用宿主拥有的无边框 Widget 窗口，并独立保存位置与尺寸；SQLite 不得在 Slint 主线程读写。Widget 必须提供不会与拖动命中冲突的宿主管理入口。 | 移动、缩放一个运行中实例后重启宿主；该实例按自身 `InstanceId` 恢复到原位置和尺寸，其他实例不被改写。点击“管理”打开普通应用角色的管理中心并选中来源实例。 |
| WA-WIDGET-02 | 内置与第三方 Widget 必须由同一宿主模式控制器协调编辑/展示状态；只有恢复热键可用时才启用点击穿透。 | 从第三方 Widget 点击“展示”后，所有 Widget 隐藏宿主控件并按能力开启穿透；按已注册恢复热键后全部恢复编辑控件、拖动与缩放命中。展示态新启动的实例继承当前模式。 |
| WA-WIDGET-03 | manifest `sizes` 必须约束 Widget 外窗，编辑/展示切换不得改变外窗位置或尺寸；最小尺寸下插件标题、管理/展示按钮、内容和缩放手柄不得重叠。 | 把可缩放插件拖到 manifest `min`，Windows 捕获尺寸等于声明值；标题与宿主操作仍可读可点。进入展示后捕获尺寸不变，插件内容扩展到隐藏工具栏释放的区域；恢复编辑后宿主框架重新出现。 |
| WA-CONTROL-01 | 管理中心必须始终位于置顶 Widget 之前；实例操作使用明确的状态与危险操作确认，不得被新创建 Widget 遮挡。 | 管理中心内停止并重新启动实例后，新 Widget 正常出现但控制面仍在前景。运行态只提供停止；停止态提供启动和删除；删除先进入仅含“确认删除/取消”的确认态。 |
| WA-CONTROL-02 | 用户选择安装版本时，管理中心必须解释其发布者、安装来源、签名信任和宿主能力声明，且开发包不得伪装成受信分发。 | 插件详情显示 publisher 名称与 id、来源路径、`已验证签名` 或 `未签名 · 仅限本地开发`，并枚举 manifest capability；信任状态使用不同视觉提示。 |
| WA-CONTROL-03 | 管理中心和运行时 Widget 的宿主操作必须向 Windows UI Automation 暴露稳定语义，并提供可见键盘焦点。 | 操作暴露为带名称和 default action 的 button，插件/实例暴露为带组合标签的 list item；鼠标第一次单击立即激活，Tab 可按视觉顺序到达当前操作，焦点环清晰，Space/Enter 可激活。运行时 Widget 的“展示”“管理”同样是 button，Tab 顺序为展示→管理；空格进入展示或打开管理中心。删除、卸载和版本切换确认态进入时接管安全焦点，Escape 取消且不执行操作。当前选择使用边框/背景并在无障碍标签中说明。独立 Slint 指针测试同时覆盖列表选择、操作单击和单击后 Space 激活。 |
| WA-CONTROL-04 | 管理中心不得用无解释的大面积空白表达“未选择”或“无可配置项”。 | 未选择时展示插件、实例两条下一步引导；选中无配置实例时明确说明无需配置，并提示仍可启动、停止、切换版本或删除。最小窗口尺寸下引导不遮挡底部操作栏。 |
| WA-CONTROL-05 | 实例启动失败时必须提供稳定错误代码、面向用户的恢复建议和原地重试，不得把本地路径或运行时原始错误直接显示给用户。 | 临时移走实例绑定的精确安装版本后，管理中心显示分类故障卡与稳定代码；恢复安装后无需重启宿主即可通过键盘重试恢复 Widget。顶部通知只显示稳定代码和通用操作提示。 |
| WA-CONTROL-06 | 启动或重试被后台监督器接受后必须立即提供处理中反馈，并阻止用户在结果返回前重复触发。 | 重试入队后 observed 状态同步切为 `starting`、错误码清空、重试按钮消失，详情区显示“正在启动插件”；命令队列拒绝请求时保留原失败状态。最终由 worker 更新为 `running` 或新的脱敏失败状态。 |
| WA-DIAG-01 | 管理中心未选择插件或实例时必须解释当前宿主平台、关闭管理中心的后果、恢复编辑入口和完全退出入口，不得让用户通过试错理解宿主生命周期。 | Windows 空状态显示单实例宿主正在运行；明确说明关闭管理中心后 Widget 与托盘继续运行，并给出 `Ctrl+Shift+E` 与通知区域托盘退出入口。其他平台显示各自准确的降级说明，不复用 Windows 托盘文案。 |
| WA-DIAG-02 | 管理中心必须提供可复制的脱敏诊断摘要，帮助内部 Alpha 用户报告插件生命周期问题；摘要不得成为配置、账户、凭证或本地路径的旁路。 | 常驻“诊断”入口显示宿主版本、安装与实例计数，以及最多 32 个实例的插件 id、版本、期望/观测状态、稳定错误码和 Connection 分类。输出小于 8 KiB，不包含 canonical config、account identity、provider、CredentialRef、secret 或本地路径。复制成功后提供短时视觉反馈，且复制不要求用户手工选中文本。 |
| WA-SETTINGS-01 | Windows 管理中心必须提供当前用户级开机启动设置；登录启动不得在没有运行实例时打扰用户，也不得绕过单实例和托盘生命周期。 | 常驻“概览”入口让用户从任意插件或实例详情返回设置卡。开关读取并写入 `HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run` 的精确 `Floatile` 值，命令引用当前 EXE 并附加 `--background`。缺失为关闭，路径不匹配显示“修复”，读取失败显式禁用。`--background` 在零运行实例时只保留单个托盘宿主；普通二次启动仍唤醒管理中心且进程数保持 1。有运行实例时恢复 Widget；待 L2 授权实例仍显示管理中心。 |
| WA-PERM-01 | 从 Installation 创建实例前必须向用户再次展示信任状态、声明的宿主能力与风险等级，并要求明确确认；查看插件详情不得隐式创建实例或改变运行授权。 | 点击“创建实例”只进入确认态且不写数据库；确认态默认键盘焦点位于“取消”，Escape 或取消保持实例数不变；只有点击“确认创建”才创建 stopped 实例，启动时仍由 Broker 和安装复核执行默认拒绝。L0 与 L2 使用 Capability Registry 的风险元数据解释，不把创建确认伪装成敏感能力会话授权。 |
| WA-PERM-02 | L2 敏感能力不得仅凭 manifest 或持久 desired state 静默激活；每个宿主会话的每次激活都需要绑定精确实例指纹的一次性确认。 | 未授权启动显示 `FPERM_SESSION_REQUIRED` 且不创建 Widget、不推进 generation；“授权并启动”只放行当时所见 plugin/version/digest/config 的下一次启动尝试。退出宿主后授权失效；冷启动主动显示管理中心并再次询问。安装或配置在确认后发生变化时旧授权不得复用。 |
| WA-SURFACE-01 | 内建时钟只作为没有任何持久化插件实例时的首次体验占位；存在运行实例时仅显示插件 Widget，只有停止实例时直接显示管理中心，不得让内建时钟与真实插件形成两个并行产品入口。 | 分别以零实例、运行实例、仅停止实例启动宿主，核对欢迎 Widget、插件 Widget、管理中心三种互斥启动表面。 |
| WA-SURFACE-02 | 插件或控制中心启动模式不得为 Slint 事件循环、托盘或恢复热键创建隐藏的内建时钟原生窗口。Windows 恢复热键必须注册到 UI 线程消息队列，继续由同一 winit 消息钩子派发。 | 运行实例冷启动后 Windows 只枚举真实插件 Widget；窗口捕获只返回插件表面。点击“展示”进入穿透后，线程级候选热键恢复全部运行 Widget 的编辑模式；退出时注销同一线程注册。 |
| WA-RENDER-01 | 作者检查、预览与持久实例必须消费同一 UI IR 到宿主 Window 的生成语义；组合状态页不得在真实窗口中才暴露编译错误。 | `floatile check` 在报告 UI 阶段成功前，使用正式 renderer 生成完整宿主 Window 并由 Slint 无显示编译；嵌套 loading/error/empty/content 分支通过同一编译门。失败返回稳定 `FCHECK_UI_RENDER` 或 `FCHECK_UI_COMPILE`，不生成可安装包。 |
| WA-CONN-01 | 需要外部数据的实例必须在管理中心显示实例级 Connection 就绪状态，不得把“没有普通配置字段”等同于“无需配置”。 | 选择声明 HTTPS 模板的实例时显示已授权 Connection 数；零授权明确解释请求会安全失败、授权是实例级且凭证不进入 guest State。读取 grant 失败不得假装为已授权。 |
| WA-CONN-02 | stopped 实例必须能从管理中心创建 Connection、保存凭证并原子授予实例，也必须能经安全确认撤销；运行实例不得热换敏感连接。 | provider、非秘密账户标识与遮罩 secret 全部有效时才允许保存；secret 入队后立即清空 UI。保存失败不留下孤立 Connection；撤销默认焦点位于取消，确认后移除 grant，共享 Connection 保留，无引用 Connection 与其凭证一并删除。 |
| WA-CONN-03 | Windows 凭证必须由宿主安全持久化，SQLite、插件 State、日志和控制面快照均不得包含明文；宿主重启后仍可按 CredentialRef 读取。 | 优先使用当前登录会话的 Windows Credential Manager；其不可用时使用 Windows DPAPI 加密后存入 `%APPDATA%\\floatile\\credentials`，文件名为 reference 的 SHA-256。自动测试用新 vault 实例读取、删除并确认密文不含 secret；Windows 桌面创建、冷启动恢复、撤销与文件清理通过。 |
| WA-CONN-04 | guest 不得依赖或观察宿主全局数据库 Connection ID；运行时只暴露实例作用域内的稳定 handle。 | supervisor 按实例 grant 顺序建立从 1 开始的 handle 表；未 grant 的真实 ID 即使被猜中也返回 `ConnectionNotGranted`。HTTPS 健康回调使用真实宿主 ID。当前管理入口最多创建一个 grant，因此参考插件使用 handle 1 不依赖数据库分配顺序。 |
| WA-CONN-05 | 管理中心必须展示 Connection 的脱敏身份与健康状态，并为失效凭证提供不破坏 grant 的恢复操作。 | 仅显示 provider、非秘密 account identity 与 `unknown/healthy/degraded/unavailable/missing` 分类，不显示 provider 原始错误。stopped 实例可在遮罩输入框更新凭证：先写入新的 CredentialRef，再原子切换数据库引用并重置健康状态为 `unknown`，最后清理旧凭证；任何前置失败都不得破坏仍有效的旧引用。运行实例只显示“停止后可更换”，不得出现更新或撤销动作。 |

## 2. 当前切片

WA-HOST-01 建立单实例所有权和重复启动拒绝。WA-HOST-02 建立 Windows 托盘、管理窗口重新打开和
显式退出的最小闭环。WA-HOST-03 将 Widget 与管理窗口的 Windows 样式、任务栏与 Alt+Tab 语义分开。
WA-HOST-04 为托盘显式退出增加最终布局保存和有界 worker 清理。WA-HOST-01 的重复启动唤醒使用
Windows 当前会话中的手动复位命名事件：第二进程发现命名互斥体已存在后只发出 activation 并退出，
主进程 UI 定时器非阻塞消费一次请求，显示并重新提升管理中心。句柄与 `unsafe` 仅存在于
`floatile-platform`。

WA-INSTALL-01 建立 Windows 内部 Alpha 的本地开发包安装入口。`floatile-platform` 封装原生 common
dialog 和不透明 owner，Shell 在短生命周期 worker 调用并把结果回投 Slint；安装工作继续由既有控制面
worker 调用统一包校验与原子安装核心。操作 notice 保持到下一次操作结果，不会在 500ms 刷新时消失。
WA-INSTALL-02 保留 Installation 的不可变、并存语义：实例引用按插件 id、语义版本与内容 digest 三者精确
匹配；任何引用都会阻断卸载。无引用版本在确认后先同文件系统 rename 到宿主忽略的 tombstone，再递归
清理，因此崩溃或清理失败不会暴露半删除安装。WA-INSTALL-03 使用只追加的 SQLite v9 migration 保存
通用版本换绑审计；Shell worker 在换绑前
复核精确目标、对 trusted 安装重新验签，并用目标 config schema 校验现有 canonical config。换绑确认明确
提示权限采用目标版本声明，并按 capability 稳定列出新增、移除和参数变化；Connection grant 不随版本
自动变化。受信 publisher 注册交互仍属于后续切片。

WA-WIDGET-01 的运行时 Window 由 renderer 生成的宿主壳拥有，插件内容只嵌入其中。拖动或缩放结束时
UI 线程把已验证 `WidgetLayout` 非阻塞投递到容量 64 的 supervisor 队列，SQLite 由 supervisor worker
独占写入；启动动作在后台读取该 `InstanceId` 的布局，再由 UI 线程应用平台窗口位置和尺寸。顶部
拖动命中显式排除右侧管理按钮区域；管理按钮只打开控制面并选中当前实例，删除仍由控制面承载。

WA-WIDGET-02 把 renderer 生成窗口的 `host_edit_mode`、`host_show` 回调和原生点击穿透接入既有
`ShellController`。模式切换由宿主广播到当前运行时会话；新会话在插入监督器前应用当前模式。
展示态不响应宿主拖动/缩放命中，`Ctrl+Shift+E`（或 Windows 注册成功的候选恢复热键）统一恢复。
Windows 插件/控制中心启动模式使用 `RegisterHotKey(NULL, ...)` 绑定 Slint UI 线程消息队列，不再为
热键或事件循环实例化内建 Clock 的 HWND；Win32 注册、注销和 `unsafe` 仍只位于
`floatile-platform`。零实例欢迎模式继续把热键绑定到真实欢迎 Widget HWND。

WA-CONTROL-01 使用固定宽度、宿主拥有的主操作/普通/危险按钮。管理中心收到实例 observed 状态变化后，
通过 `floatile-platform` 的显式重新提升操作刷新同级 topmost Z 序；它不逐帧调用原生窗口 API。
删除确认只在 stopped 实例出现，取消不发送后台命令。
运行时会话释放时显式 `hide()` interpreter Window，再把 wasm worker 交给异步 reaper；因此 stopped
状态不会遗留仍更新的原生窗口，所有实例停止且管理中心关闭后可进入真正的仅托盘状态。
WA-CONTROL-02 的展示数据只来自已经过安装目录完整性复核的 manifest 与 `install.json`，不把来源文本或
publisher 自声明当作信任证明；`trusted` 仍必须在运行前按宿主 trust store 重新验签。
WA-CONTROL-03 的自绘 `HostAction` 仍维持统一视觉，但使用 Slint `FocusScope`、button role、label 与
default action 提供平台语义；列表行使用 list-item role，选择状态同时通过视觉边框和组合标签表达。
WA-DIAG-01 在没有选择项时展示由 Rust 按目标平台注入的宿主运行说明，Slint 只负责布局，不把 Windows
生命周期假设写死在跨平台界面中。该说明不读取数据库、不调用 OS API，也不改变现有关闭或退出行为。
WA-DIAG-02 的摘要由控制面快照派生，只使用已经脱敏的生命周期字段，并在 Rust 中同时执行条目数和
字节预算限制。Slint 的只读文本框只承担显示与系统剪贴板交互；复制动作分帧完成焦点、全选和复制，
避免按钮点击焦点导致空复制，并提供短时“已复制”反馈。摘要生成不读取 vault、配置值或安装来源路径。
WA-SETTINGS-01 的注册表读写只存在于 `floatile-platform::autostart`，控制面 worker 后台读取状态和
执行切换，Slint 线程只发送有界命令。Run 值始终使用带引号的当前可执行文件路径和固定
`--background` 参数；不接受插件或用户提供的命令片段。其他平台返回 `Unsupported`，读取错误返回
`Unavailable`，界面禁用开关而不假装成功。后台模式不削弱 L2 会话授权：只在 desired-running 数为
零时压制欢迎/控制中心，有敏感运行实例时仍保留授权入口。
WA-PERM-01 复用已经过安装完整性校验的 trust、capability 与 Capability Registry 风险元数据建立
创建前确认；确认只创建当前实例，不放宽 manifest grant，也不替代 L2 会话授权或绕过运行时
`PermissionBroker`。取消按钮接管安全默认焦点。
WA-PERM-02 由 supervisor worker 在推进 generation 与创建 runtime 前复核精确 Installation manifest。
授权命令只携带实例 ID，但 worker 从已经隔离的 `InstanceFingerprint` 建立一次性 token；下一快照的
plugin、version、digest 或 config 任一变化都会拒绝该 token。token 不落 SQLite，进程退出自然失效。
存在 desired-running 的 L2 实例时，启动表面强制包含普通管理中心，同时继续运行其他已获准的 L0 Widget。
WA-RENDER-01 把此前只在持久实例启动时执行的 renderer/Slint 编译前移到作者 `check`：检查仍不创建
原生窗口，但编译的是与 preview 和 supervisor 相同的完整宿主 Window 源码。renderer 的条件分支
统一产生独立组件边界，因此 SDK `page_state` 生成的嵌套 `If` 既保持优先级，也符合 Slint 条件元素语法。
Windows 实机已验证 AI Balance 经 L2 会话授权后显示为统一无边框 Widget，而不是独立系统标题栏窗口。
WA-CONN-01 复用 PP-M5 的 Installation manifest 与实例级 Connection grant，不从插件 State 推测连接
状态。控制面 worker 后台读取 grant，Slint 主线程只消费计数与脱敏说明；AI Balance 的 Windows 实机
选择态已验证零授权时显示“尚未授权连接”，不再显示误导性的“无需配置”。
WA-CONN-02 的 secret 只短暂存在于密码输入框和实现清零的 `SecretInput`，后台 worker 先写 vault，
再创建 Connection 与 grant；数据库步骤失败会删除刚写入的凭证。撤销只允许 stopped 实例，并用二次
确认保护；最后一个 grant 删除后同时清除 Connection 与凭证，共享引用则保持。
WA-CONN-03 的 `CredentialVault` 仍是 runtime 与控制面的共同宿主句柄。Windows Credential Manager
在普通登录会话可用时为首选；没有凭证登录会话的自动化/服务上下文会返回 Win32 1312，此时以
`CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN` 生成 DPAPI 密文并依赖当前用户
`%APPDATA%` ACL 限制文件访问。该降级比用户范围 DPAPI 弱，必须在 Beta 威胁模型中复核，但不会
退回明文或会话内存，也不改变 guest 只能使用不透明 CredentialRef 的边界。
WA-CONN-04 由 `HttpsService::new_with_handles` 分离 guest handle 与持久 ID；缓存键和 guest 请求使用
局部 handle，健康状态 listener 使用真实 ID。服务测试以真实 ID 7、guest handle 1 验证：7 在 guest
侧被拒绝，1 可执行，回调仍报告 7。多连接的具名 slot 尚未形成版本化契约，当前不伪装为已完成。
WA-CONN-05 的健康回写由 supervisor 的有界非阻塞队列交给 SQLite owner worker，Slint 线程只读取
脱敏快照。凭证更新沿用 stopped 门禁和实现清零的 `SecretInput`；新 vault entry 写入成功后才调用
`rotate_credential` 增加 generation 并把健康状态重置为 `unknown`。数据库切换失败会删除新 entry，
切换成功后才删除旧 entry，因此恢复操作不会先销毁当前可用凭证。共享 Connection 的所有 grant
会共同使用新 generation，界面不得把更新描述成只影响当前 guest 的私有配置。

Windows 使用 `Local\\` 命名互斥体，范围限定为当前登录会话；其他平台本切片返回显式不支持并维持
现有启动行为。平台资源与 `unsafe` 只存在于 `floatile-platform`，`floatile-shell` 只消费获取结果。
