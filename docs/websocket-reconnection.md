# WebSocket Relayer 心跳检测与断点续传方案

## 问题背景

本服务通过 WebSocket 连接 relayer 服务，实时获取 PDS（Personal Data Server）中的新增数据。实际运行中发现两个问题：

1. **连接卡住**：WebSocket 连接有时会长时间无响应，导致无法接收新数据
2. **消息丢失**：连接断开后重连，会丢失中断期间新增的 commit 消息

## 解决方案

### 1. 心跳检测与自动重连

**原理**：使用 `tokio::time::timeout` 对 `stream.next()` 设置超时（默认 60 秒）。如果超时时间内未收到任何消息（包括 Ping/Pong），则判定连接已卡住，主动断开并触发重连。

**实现要点**：
- 直接监听底层 WebSocket stream，而非通过抽象层
- 正确处理 `Message::Ping` / `Message::Pong`：收到后重置计时器，不断开连接
- 正确处理 `Message::Close`：记录日志后返回，触发外层重连
- 使用指数退避（1s → 2s → 4s ... 上限 30s）避免对故障 relayer 造成压力

```rust
// src/relayer/subscription.rs
loop {
    let result = timeout(
        Duration::from_secs(self.heartbeat_timeout_secs),
        self.stream.next(),
    ).await;

    match result {
        Ok(Some(Ok(Message::Binary(data)))) => { /* 处理数据 */ }
        Ok(Some(Ok(Message::Ping(_)))) | Ok(Some(Ok(Message::Pong(_)))) => {
            continue; // 心跳消息，重置计时器
        }
        Ok(Some(Ok(Message::Close(frame)))) => {
            return Ok(()); // 连接关闭，触发重连
        }
        Err(_) => {
            return Err(eyre!("Heartbeat timeout: ...")); // 连接卡住
        }
        // ...
    }
}
```

### 2. 断点续传（Cursor-based Resume）

**原理**：AT Protocol firehose 的每个 commit 消息都包含 `seq`（序列号）。重连时通过 URL 参数 `?cursor={seq}` 告诉 relayer 从哪个位置继续发送，避免漏掉中断期间的消息。

**实现要点**：
- `last_seq` 存储在 `Arc<AtomicI64>` 中，线程安全
- `last_seq` 在 `handle_commit` 处理**成功后**才更新，确保不跳过未成功处理的消息
- 重连时读取 `last_seq`，构造带 cursor 的 URL：`wss://relayer?cursor=12345`

```rust
// 重连逻辑
let cursor = if handler.last_seq() > 0 {
    Some(handler.last_seq())
} else {
    None
};
RepoSubscription::new(&relayer, cursor).await
```

```rust
// handle_commit 中，处理成功后才更新 cursor
async fn handle_commit(&self, commit: &Commit, seq: i64) -> Result<()> {
    // ... 所有数据处理逻辑 ...
    
    self.last_seq.store(seq, Ordering::SeqCst);
    Ok(())
}
```

## 当前实现状态

当前实现使用**数据库持久化** cursor 存储：
- ✅ 同进程内重连不会丢失消息
- ✅ 进程重启后从断点继续（读取数据库中的 cursor）
- ✅ 每 10 个 commit 异步持久化一次，平衡性能与数据安全

### Migration

执行 `migrations/001_cursor_state.sql` 创建表：

```sql
CREATE TABLE IF NOT EXISTS cursor_state (
    id      SERIAL PRIMARY KEY,
    name    VARCHAR(64) NOT NULL UNIQUE,
    seq     BIGINT NOT NULL DEFAULT 0,
    updated TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO cursor_state (name, seq) VALUES ('relayer', 0)
ON CONFLICT (name) DO NOTHING;
```

### 持久化策略

每 10 个 commit 异步写入数据库一次，平衡性能与数据安全：

```rust
async fn handle_commit(&self, commit: &Commit, seq: i64) -> Result<()> {
    self.last_seq.store(seq, Ordering::SeqCst);
    
    if seq % 10 == 0 {
        CursorState::set_seq(&self.db, "relayer", seq).await.ok();
    }
    Ok(())
}
```

如需更严格的数据保证，可调整为每次 commit 都写入。


## 配置项

如需调整心跳超时时间，可在创建订阅时设置：

```rust
let sub = RepoSubscription::new(&relayer, cursor).await?
    .with_heartbeat_timeout(120); // 120 秒
```

## 监控建议

建议添加以下指标监控：
- `relayer_reconnect_count`：重连次数（Counter）
- `relayer_last_seq`：当前处理到的序列号（Gauge）
- `relayer_commit_latency`：commit 处理延迟（Histogram）
