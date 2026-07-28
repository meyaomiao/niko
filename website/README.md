# Niko 官网

`https://niko-ai.cc` 的 Cloudflare Pages 站点。官网保持原生 HTML、CSS 和 JavaScript；账户请求经同域 Pages Functions BFF 转发到统一账户服务。

```text
浏览器
  -> niko-ai.cc/api/niko/v1/*
  -> Cloudflare Pages Functions
  -> momotoken /api/niko/v1/*
```

浏览器不会直接请求 momotoken，也不会持有上游 `session_token`。Niko 不保存用户、余额、订单或账单副本。

## 本地运行

```bash
cd website
npm install
npm run check
npm run dev
```

`npm run check` 会检查浏览器脚本、Functions 语法并构建 `dist/`。本地 Functions 环境变量可放在未提交的 `.dev.vars` 中。真实 Turnstile 联调应使用允许本地域名的测试 Site Key；只做页面 Mock 时可设置 `NIKO_LOCAL_TURNSTILE_DISABLED=true`。

## Pages 环境变量

必须配置：

- `MOMOTOKEN_NIKO_API_ORIGIN`：上游地址，只接受 HTTPS 生产地址。
- `NIKO_BFF_SECRET`：与 momotoken 相同的 HMAC 密钥，至少 32 字节，必须使用 Pages Secret。
- `NIKO_TURNSTILE_SITE_KEY`：前端公开的 Turnstile Site Key。
- `NIKO_PAYMENT_ALLOWED_ORIGINS`：允许返回给浏览器的支付页 Origin，逗号分隔，必须包含协议，例如 `https://pay.example.com`。

可选配置：

- `NIKO_SESSION_COOKIE_MAX_AGE`：Cookie 最长秒数，默认 7 天，上限 30 天；不会延长上游 Session。
- `NIKO_UPSTREAM_TIMEOUT_MS`：上游超时，默认 10 秒，范围 2–20 秒。
- `NIKO_LOCAL_TURNSTILE_DISABLED`：仅 `localhost` 本地页面 Mock 使用，生产域名无效。

## 浏览器会话

- Session：`__Host-niko_session`，`Secure; HttpOnly; SameSite=Lax; Path=/`。
- CSRF：`__Host-niko_csrf`，`Secure; SameSite=Lax; Path=/`。
- 所有非 GET 请求必须同时满足同源 `Origin` 和 `X-CSRF-Token` 双提交校验。
- 登录成功后，BFF 从上游 JSON 中取出 `session_token`、`csrf_token` 写入 Cookie，再删除这两个字段后返回浏览器。
- 上游 `401` 或登出成功会清除两个 Cookie。
- API 响应统一 `Cache-Control: no-store`，不返回跨域 CORS 许可。

## 上游服务签名

每次 BFF 请求均发送：

- `X-Niko-Timestamp`：Unix 秒。
- `X-Niko-Nonce`：16 字节随机数的十六进制字符串。
- `X-Niko-Client-IP`：只取 Cloudflare `CF-Connecting-IP`，并纳入签名。
- `X-Niko-Session`：登录态请求携带上游 Session Token。
- `X-Niko-CSRF`：登录后的写请求携带上游 CSRF Token。
- `Idempotency-Key`：创建充值订单时携带。
- `X-Niko-Signature: <hex hmac>`。

签名原文使用 UTF-8，各行以 `\n` 拼接，末尾不加换行：

```text
<HTTP_METHOD>
<path_and_query>
<body_sha256_hex>
<timestamp>
<nonce>
<client_ip>
<session_token_or_empty>
<csrf_token_or_empty>
<idempotency_key_or_empty>
```

签名算法为 `HMAC-SHA256(NIKO_BFF_SECRET, canonical_text)`，输出小写十六进制。签名使用实际转发的原始 JSON 字符串；GET 空 Body 使用空字符串的 SHA-256。上游必须校验时间窗口、Nonce 防重放、Body 摘要、签名和客户端 IP 字段的一致性。

## BFF 白名单

浏览器与上游路径一致，BFF 只开放：

- `POST /api/niko/v1/auth/register`
- `POST /api/niko/v1/auth/login`
- `GET /api/niko/v1/auth/session`
- `POST /api/niko/v1/auth/logout`
- `GET /api/niko/v1/account/me`
- `POST /api/niko/v1/account/email/send-code`
- `POST /api/niko/v1/account/email/bind`
- `GET /api/niko/v1/wallet/summary`
- `GET /api/niko/v1/wallet/topups?cursor=&limit=`
- `GET /api/niko/v1/wallet/consumptions?cursor=&limit=`
- `GET /api/niko/v1/wallet/topup-options`
- `POST /api/niko/v1/wallet/topup-orders`
- `GET /api/niko/v1/wallet/topup-orders/:order_id`

另有仅由 BFF 提供的 `GET /api/niko/v1/config`，用于初始化 CSRF Cookie 和公开 Turnstile Site Key。

## 数据契约

接口可直接返回数据，也可使用 `{ "data": ... }` 包装。错误建议统一为：

```json
{
  "error": {
    "code": "STABLE_ERROR_CODE",
    "message": "可安全展示给用户的中文说明"
  }
}
```

关键响应字段：

- 账户：`id`、`username`、`email`、`created_at`。
- 余额：`balance_quota` 必须是字符串；前端展示 `display_balance`、`display_currency` 和 `updated_at`，不会自行把额度换算为金额。
- 列表：`items`、`next_cursor`，Cursor 是不透明字符串。
- 充值记录：`id` 或 `order_id`、`display_amount`、`display_currency`、`payment_channel`、`status`、`created_at`、`paid_at`。
- 消费明细：`id`、`model`、`amount_quota` 字符串、`display_amount`、`display_currency`、`prompt_tokens`、`completion_tokens`、`created_at`。
- 充值选项：`options[]` 至少包含 `id`、`display_amount`，`channels[]` 包含 `id/code` 与 `name/label`。
- 创建订单：返回 `order_id` 和 `payment_url`；`payment_url` 必须是 HTTPS 且 Origin 在 BFF 白名单。
- 查询订单：返回 `order_id`、`status`；状态为 `pending`、`success`、`failed`、`expired`、`partially_refunded` 或 `refunded`。

创建充值订单必须带 `Idempotency-Key`。浏览器只能提交 `option_id`，或 `amount + currency`，以及 `payment_channel`；BFF 将十进制金额精确转换为 `amount_minor`，并拒绝余额、到账额度、`quota_to_add` 和浏览器传入的 `return_url`。momotoken 固定生成 Niko 的 `/payment/return/` 地址；回跳页面只按 `order_id` 查询，上游订单状态只能由可信支付回调更新。

Turnstile action 固定为：注册 `niko_register`、登录 `niko_login`、发送邮箱验证码 `niko_email_code`。momotoken 同时校验 action 和 hostname。

## 构建与部署

构建脚本从 `brand/niko-logo-kit-c` 复制品牌资产并生成 Open Graph 图片。部署命令仍为 `npm run deploy`，但开发和联调阶段不要运行该命令。
