---
active: true
iteration: 1
max_iterations: 500
completion_promise: "DONE"
initial_completion_promise: "DONE"
started_at: "2026-04-17T08:28:36.449Z"
session_id: "ses_265984c76ffethVyuH7b42eFGh"
ultrawork: true
strategy: "continue"
message_count_at_start: 39
---
本服务会以websocket的方式连接relayer服务，以获取pds中新增的数据。但是现在有两个问题：1. 这个websocket连接有的时候会卡住，需要增加类似心跳的检测机制和自动重连机制。 2. 重连之后，需要从上次获取内容的点，继续获取更新内容，而不是漏掉连接中断期间新增的消息。
