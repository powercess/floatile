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
| WA-HOST-01 | Windows 桌面宿主必须在当前用户会话中保持单实例；第二次启动必须在创建窗口、打开数据库或启动插件前退出，不得影响首个实例。 | 同一 Windows 会话连续启动两次 `floatile-shell`，只保留首个宿主进程及其 Widget；首个实例继续响应。 |
| WA-HOST-02 | 宿主必须提供系统托盘入口；管理中心关闭后收起到托盘，托盘提供重新打开与显式退出。 | Windows 通知区显示 Floatile 图标；左键或菜单“打开 Floatile”显示插件与实例管理窗口；关闭该窗口后宿主与 Widget 继续运行；菜单“退出 Floatile”结束宿主。 |
| WA-HOST-03 | 管理中心与 Widget 必须使用不同窗口角色；Widget 不进入任务栏/Alt+Tab，管理中心按普通应用窗口呈现。 | Widget 保持无边框置顶但不出现在任务栏和 Alt+Tab；管理窗口显示 Windows 系统标题栏，可从任务栏和 Alt+Tab 切换。 |
| WA-HOST-04 | 显式退出必须有界停止插件、保存布局、释放平台资源并关闭数据库；单个插件不得无限阻塞退出。 | 后续切片。 |

## 2. 当前切片

WA-HOST-01 建立单实例所有权和重复启动拒绝。WA-HOST-02 建立 Windows 托盘、管理窗口重新打开和
显式退出的最小闭环。WA-HOST-03 将 Widget 与管理窗口的 Windows 样式、任务栏与 Alt+Tab 语义分开。
第二次启动主动唤醒管理中心仍需要跨进程唤醒协议，不在当前闭环中伪造成功。

Windows 使用 `Local\\` 命名互斥体，范围限定为当前登录会话；其他平台本切片返回显式不支持并维持
现有启动行为。平台资源与 `unsafe` 只存在于 `floatile-platform`，`floatile-shell` 只消费获取结果。
