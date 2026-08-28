# ADR-0005：插件包签名、发布者信任与更新防降级

> 状态：Accepted
> 日期：2026-08-28
> 决策者：Floatile 项目

## 背景与需求

PP-M8 要求安装与升级能够解释来源、发布者、权限变化和失败原因，并覆盖篡改、降级、撤销与回滚。
现有 `.floatile` 校验和 `content_digest` 能发现安装前后内容变化，但不能证明发布者身份，也不能阻止
攻击者用签名正确的旧版本替换新版本。签名格式、摘要文件集合和信任锚一旦发布就形成长期兼容承诺，
必须先于实现固定边界。

本 ADR 不解除 NFR-LEGAL-01：许可 ADR 通过前仍不得对外分发宿主、SDK 或插件包。

## 候选方案

### A. manifest 内嵌裸 Ed25519 签名

拒绝。签名字段会造成自引用摘要问题；裸签名没有 payload type 域分离，容易被跨协议误用；对密钥轮换
和多签名也缺少清晰扩展边界。

### B. detached DSSE envelope + Ed25519

采用。DSSE 的 PAE 将 payload type 与 payload bytes 一起认证，不依赖 JSON canonicalization；Ed25519
使用固定 32 字节公钥和 64 字节签名。`signature.json` 只承载 envelope，不参与被签名内容摘要。

参考规范：

- RFC 8032：<https://www.rfc-editor.org/info/rfc8032/>
- DSSE protocol：<https://github.com/secure-systems-lab/dsse/blob/master/protocol.md>
- TUF rollback guidance：<https://theupdateframework.io/docs/overview/>

### C. 直接采用完整 TUF/Sigstore 在线基础设施

当前不采用。它们适合仓库元数据、透明日志和在线发布，但会把网络、PKI、账户和运营体系同时带入首个
本地验证切片。格式保留未来把 trusted key 来源升级为 TUF/Sigstore 的空间，不在包校验器中硬编码服务。

## 决策

### 1. 签名文件与摘要集合

- 包内签名文件固定为根路径 `signature.json`；manifest v1 不增加 `signature` 字段。
- `content_digest` 的签名输入是通过全部包安全校验的规范普通文件集合，唯一排除
  `signature.json`。任何其他文件的增删、重命名或字节变化都改变摘要。
- 安装完整性摘要继续覆盖落盘的全部普通文件，包括 `signature.json`；签名摘要与安装摘要是两个不同
  目的的值，不得混用。
- 签名必须在解析 manifest、UI IR 或 WASM 之前可独立验证，但 zip 路径、大小、重复和压缩预算必须先
  通过，避免签名解析绕过不受信输入预算。

### 2. Envelope v1

`signature.json` 使用有界 DSSE JSON envelope：

```json
{
  "payloadType": "application/vnd.floatile.package-digest.v1",
  "payload": "<base64 32-byte SHA-256>",
  "signatures": [
    { "keyid": "<lowercase SHA-256 public-key hex>", "sig": "<base64 Ed25519>" }
  ]
}
```

- verifier 必须严格要求上述 payload type、32 字节 payload、至少一个且有上限的 signature；未知字段
  默认拒绝，格式升级使用新的 payload type。
- Ed25519 签名对象是 DSSE PAE bytes，不是 payload、base64 文本或 JSON envelope。
- `keyid` 只是候选密钥查找提示，不是身份或授权事实；它必须等于公钥原始 32 字节的 SHA-256 hex，
  但只有宿主信任存储中的公钥可以建立 trust。
- v1 只接受 Ed25519；不从不受信 envelope 读取或协商 `alg`。

### 3. 发布者与信任

- manifest `publisher.id` 是被摘要覆盖的声明，不是自证身份。
- 宿主信任存储维护 `publisher id → trusted public keys + state`；包内公钥、证书或 keyid 不得自动成为
  trust anchor。
- 至少一个有效签名必须来自该 publisher 当前 trusted key，且 manifest publisher 与 trust binding
  精确匹配，才能得到 `trusted`。
- trust outcome 使用 `unsigned | untrusted | trusted | revoked` 显式表达。开发策略可以允许
  `unsigned/untrusted` 安装，但必须标记来源并禁止静默提升为生产信任；分发策略默认拒绝。
- 密钥轮换由宿主信任存储更新，或由旧、新 trusted key 的重叠签名证明；仅由新 key 自签不能轮换。

### 4. 权限、升级与回滚

- 签名只证明完整性和受信发布者，不授予 capability。Permission Broker 和用户 grant 继续独立、
  deny-by-default。
- 更新计划必须比较精确插件 id、publisher、semver、内容摘要、engine/UI 兼容性和权限集合。
- 新增 capability、扩大 scope/配额或改变 Connection 绑定必须重新确认；降权可以自动接受但要审计。
- 宿主按 `publisher + plugin id` 记录最高已接受版本及摘要。较低版本、同版本不同摘要、已撤销 key 或
  publisher 改变默认拒绝。
- 显式回滚必须引用已验证的历史 Installation、记录原因并保持数据 migration 兼容；回滚不是删除
  anti-rollback 状态，也不能让旧权限重新扩大。

### 5. 资源与失败语义

- envelope、字符串、签名数量和 base64 解码结果都有硬上限；解析失败不得回显签名字节、公钥或宿主
  trust-store 路径。
- 稳定错误至少区分：missing、malformed、unsupported payload、digest mismatch、unknown key、
  invalid signature、publisher mismatch、revoked、rollback、permission confirmation required。
- 任一失败必须在安装目录原子落盘前发生，并证明零残留；校验失败不能影响已安装版本。

## 后果

收益：签名无 JSON canonicalization、自引用或算法降级；publisher identity、权限和升级策略不会被一个
“签名有效”布尔值混淆；未来可以替换 trust-key 配送方式而不改变包内容签名。

代价：需要新的纯领域 trust/signature 类型、Ed25519 依赖、host-owned trust store、anti-rollback
持久化和升级状态机。完整 TUF/Sigstore、在线撤销和透明日志留给后续 ADR/切片。

## 验证要求

首批实现必须覆盖已知 Ed25519 向量、DSSE PAE 域分离、签名/摘要/路径篡改、未知 key、publisher
错配、撤销、签名数量/大小预算、同版本不同摘要和降级拒绝。CLI 安装集成必须额外证明失败零残留与
既有 unsigned dev policy 的显式结果。
