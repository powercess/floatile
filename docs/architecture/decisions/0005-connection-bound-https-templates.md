# ADR-0005：Connection 绑定的 HTTPS 请求模板

> 状态：Accepted
> 日期：2026-08-27
> 关联：PP-M5、PP-G1、PP-G2、PP-G3、PP-G4、PP-G5

## 背景

PP-M5 需要让 guest 获取外部数据，但不得获得 ambient socket、DNS、TLS、原始凭证或任意 URL 能力。
现有 HTTP Broker 提案要求静态请求模板，PP-M3 则要求 Capability Registry 使用有限稳定 ID；若把每个
origin 编码成动态 capability 名，manifest schema、WIT、Broker 和作者工具将重新出现无法穷举的权限语义。
此外，ADR-0004 要求网络使用宿主 Operation，不能增加阻塞式 guest import 或通用 JSON-RPC。

## 决策

1. Capability Registry 新增固定能力 `network:https`，其参数包含规范化的精确 HTTPS origin、每分钟
   请求数、单响应字节上限与最长 timeout。V1 不支持通配 origin、裸 IP、localhost、私网或非 HTTPS。
2. manifest v1 以向后兼容的可选 `httpTemplates` 数组声明请求模板。每项包含稳定 template ID、固定
   method、固定 HTTPS URL（origin + path）、允许的 query 参数名、credential header 名、允许状态码、
   `maxBytes` 与 `timeoutMs`。模板不得包含 secret、URL userinfo、fragment 或运行时可变 host/path。
3. 插件提交请求时只提供 template ID、一个已授予当前实例的 `connection-id` 和有界 query 参数键值。
   `connection-id` 可猜测但不可伪造授权：Broker 必须在同一 submit 入口重新读取实例 grant、Connection
   health、credential generation 和 `network:https` origin scope。
4. Connection 的 `CredentialRef` 只在宿主组合层解析。Broker 从 CredentialVault 借用 secret 并注入
   模板指定 header；guest、Operation completion、审计、日志、State、Config、SQLite 和响应错误均不得
   包含 secret 或完整 header/query/body。
5. HTTPS 通过 ADR-0004 typed Operation 暴露：`submit` 立即返回 operation ID，completion 只含终态，
   `take-result` 一次性返回有界 status/body。WIT 不暴露 header、socket、DNS 或 TLS 配置。
6. V1 初始切片禁用自动 redirect。后续支持 redirect 时，每一 hop 必须重新执行 scheme/origin/DNS/
   私网校验并剥离凭证；不得启用客户端默认 redirect。
7. DNS 解析结果在连接前拒绝全部非公网地址，并固定连接到已验证 IP，同时保留原 host 进行 TLS SNI 和
   证书校验。无法安全固定解析结果时返回 `unavailable`，不得退回客户端二次解析。
8. 网络配额由 Broker 按实例维护；缓存、重试和调度只能调用同一模板 submit 路径，不得拥有绕过授权
   的内部 HTTP 执行入口。权限撤销、Connection 轮换或 generation 变化使旧结果不可投递。

## 后果

- 新 provider 通常只需增加插件模板与 Connection 配置，不需要宿主专用 API。
- manifest/WIT/SDK/Capability Registry 将发生一次联动版本演进，但保持单一事实源和类型化结果。
- 初始版本有意限制动态 REST 客户端能力；需要可变路径、POST body 或分页时，应扩展模板 schema 和
  contract vectors，而不是向 guest 暴露自由 URL。
- 平台 Keyring 不可用时 Connection 显式 unavailable；会话 vault 只支持开发和确定性测试。

## 验证要求

- contract vectors 覆盖允许请求，以及未授权 Connection/origin、未知模板、非法 query、私网解析、
  redirect、超时、取消、响应过大、限流、凭证缺失/轮换和旧 generation 丢弃。
- 恶意 fixture 必须证明 secret 不进入 guest、completion、错误、审计、日志、State、Config 或包。
- AI Balance Monitor 只能消费通用 Connection、HTTPS Operation、调度和缓存能力。
