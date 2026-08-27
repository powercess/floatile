# HTTP Broker 设计

> 状态：Proposed
> 阶段：V1 实现目标。P0/MVP 不实现网络能力（WIT 中无对应接口 = 无攻击面）。
> 本设计先行冻结，确保 V1 不会因“便利”绕过安全约束。

## 1. 目标

- 插件永远不直接获得网络句柄，只能通过宿主 Broker 发起**受控** HTTP 请求。
- 域名白名单、重定向校验、私网/localhost 单独授权、TLS 强制。
- 凭证只由宿主从 Keyring 注入，插件永远读不到原始密钥。
- 请求/响应大小、时长、频率受配额限制。
- 全链路审计（脱敏）。

## 2. 请求模板（Request Template）

插件不是「自由填 URL」，而是**注册模板**，宿主校验后绑定到 `network:` 权限：

```json
{
  "id": "weather.current",
  "method": "GET",
  "urlTemplate": "https://api.weather.example.com/v1/current",
  "queryParams": { "city": "{{city}}" },
  "headers": {
    "x-api-key": { "fromCredential": "cred://weather/apikey" }
  },
  "allowedStatus": [200],
  "maxBytes": 65536,
  "timeoutMs": 10000
}
```

- `{{var}}` 占位符由插件在调用时提供（运行时参数），但**域名、路径、header 名**在模板中固定，不可由插件参数覆盖。
- 敏感头只允许 `fromCredential`（引用 Keyring 中宿主管辖的条目）或 `fromConfig`（用户配置、非明文）。插件代码里永远没有明文 key。
- 插件调用形态：`request(templateId, params)` → 宿主校验 → 注入凭证 → 发起。

## 3. 请求处理管线

```
request(template_id, params)
  │
  ├─ 1. 模板存在 & 权限 network:<origin> 已授予
  ├─ 2. 解析 URL；校验 scheme ∈ {https, wss}
  ├─ 3. 域名白名单校验（精确/通配，见 §4）
  ├─ 4. DNS 解析 → 私网/保留 IP 校验（§5）
  ├─ 5. TLS 强制（rustls，禁用任何“跳过证书校验”路径）
  ├─ 6. 发送（reqwest + rustls + 自定义 connector 固定已解析 IP，防 DNS rebinding）
  ├─ 7. 重定向 → 每个 hop 重新走 2-5（见 §6）
  ├─ 8. 配额检查（频率/大小/时长）→ 通过则返回响应体（受 maxBytes 限制）
  └─ 9. 审计（脱敏）
```

## 4. 域名白名单

- 权限声明形式：`network:https://api.example.com/*`、`network:https://*.example.com/*`、`network:websocket:wss://stream.example.com/*`。
- 匹配规则：
  - 精确 host 或单层通配 `*.`；
  - **禁止**裸 IP、`localhost`（需单独 `network:localhost` 授权）、公共后缀通配；
  - punycode 统一后再比较；大小写归一。
- `network:localhost`（回环）单独授权、默认拒绝、L3 级处理。

## 5. 私网/保留地址防护

- 解析结果必须落在公网地址，否则拒绝。
- 保留范围清单：`10/8`、`172.16/12`、`192.168/16`、`127/8`、`169.254/16`、`0.0.0.0/8`、`100.64/10`、`::1`、`fc00::/7`、`fe80::/10`、multicast/reserved。
- **防 DNS rebinding**：reqwest 内部解析后连接同一 IP 存在 TOCTOU；改为：用 hickory（可信根）解析 → 校验 → 用 `hickory-resolver` 结果构造自定义 connector 固定连接该 IP，同时校验 TLS SNI/host 为原域名（SNI 保持域名，连接 IP 为已验证 IP）。

## 6. 重定向策略

- 使用 reqwest 自定义 `redirect::Policy`（禁用默认跟随，手动逐 hop 校验）：
  - 每 hop 重新执行域名白名单 + 私网校验；
  - 最多 N=5 跳；
  - 跨到未授权域 → 中断并审计。
- 重定向会**剥离敏感头**（凭证头只对初始模板域生效，跳转后丢弃）。

## 7. 配额与资源限制

- 单请求：`maxBytes`（响应体上限）、`timeoutMs`。
- 每实例：并发请求数（≤4）、频率（如 10/min，按权限）、单日字节总量。
- WebSocket（V1.5+）：握手同校验；帧大小限制；心跳保活；断开重连由宿主控制并重新校验。

## 8. 凭证模型

- 插件在 manifest 声明 `credentialRefs: ["cred://weather/apikey"]`（引用而非值）。
- 宿主启动/激活实例时，从 Keyring 取 `weather/apikey` 对应的秘密到内存中的安全存储（不落盘、不写日志）。
- 模板引用 `fromCredential` 时，Broker 注入；插件拿到的永远是「已注入的请求」，读不到原始值。
- Keyring 不可用（如 Linux 无 Secret Service）→ 降级为宿主管理的加密文件凭证库（显式 opt-in），或拒绝网络能力。

当前 PP-M5 基线已实现宿主 `CredentialVault` 接口与不落盘的会话 vault，用于 Broker 组合和确定性
安全测试。它不支持跨宿主重启：平台 Keyring 未接入时，重启后的 Connection 必须显式报告
`unavailable`，不得从 Config、State、环境变量或普通 SQLite 表恢复 secret。

## 9. 审计（脱敏）

- 记录：时间、plugin_id、instance_id、template_id、method、origin（域名级）、status、bytes、duration、decision。
- 不记录：URL 查询参数值、请求/响应体、任何 header 值（只记敏感头存在性）。

## 10. 与权限模型的衔接

- `network:` 权限属于 L2 高风险，激活时二次确认。
- 模板注册本身也是一次授权操作（审计 + 可在权限中心查看/撤销）。
- 撤销权限 → 已注册模板失效，无需插件配合。

## 11. 未决问题

1. 是否允许插件自定义非敏感 header 的**值**（模板内固定 vs 运行时可传）。倾向：非敏感且白名单内 header 允许运行时传值，敏感 header 只允许 fromCredential。
2. WebSocket 心跳/重连的归属（宿主负责，插件只订阅事件）。
3. 缓存响应是否引入（倾向：V1 不做服务端缓存，减少攻击面）。
