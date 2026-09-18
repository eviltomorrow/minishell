# 01 — 静默黑洞连接的存活检测与重连不再退出

**What to build:** 修复「笔记本睡眠唤醒后，SSH 会话卡死在 shell、按键无回显、无任何提示」以及「按回车重连有时候不好用」两个症状。断开检测必须**有界**（远小于内核默认的 ~15 分钟 `tcp_retries2` / 2 小时 `SO_KEEPALIVE`），检测到后弹出重连提示；重连失败不能直接退出程序，而是回到提示让用户再按一次。

**Blocked by:** None — can start immediately.

**Status:** resolved

- [x] 内核级死亡检测：`configure_liveness()` 设置 `TCP_USER_TIMEOUT(20s)` + `TCP_KEEPIDLE(30s)`/`TCP_KEEPINTVL(10s)`/`TCP_KEEPCNT(3)`，黑洞连接在 1 分钟内转为 `POLLERR`（最坏 80s 界）
- [x] SSH 层 keepalive 间隔 120s → 30s，且不再吞掉真实传输错误（原先 `let _ = session.keepalive_send()`）
- [x] 非阻塞写入不再丢按键：`flush_pending()` 自行记录已写字节数并保留余量，替代会丢失已写前缀的 `let _ = channel.write_all(..)`
- [x] 重连失败（`prepare_session` 报错）不再 `return Err` 退出，改为打印失败原因并回到重连提示；耗尽次数后带上最后一次错误
- [x] 移除 `channel.flush()`：ssh2-rs 的 `Channel::flush` 映射到 `libssh2_channel_flush_ex`，丢弃的是**接收**数据（banner/prompt），不是发送数据
- [x] 回归测试：`tests/liveness_test.rs` 覆盖 socket 选项实际生效 + 部分写入/WouldBlock 下字节零丢失 + 硬错误传播（4 个测试）

## Answer

根因（已对 libssh2 源码核实，非推测）：`libssh2_keepalive_send` 静默吞掉 `EAGAIN` 并始终返回 0（`keepalive.c`），因此 `let _ = session.keepalive_send()` 既发不出探测、也拿不到任何存活信号；循环内没有任何应用层存活检测；`SO_KEEPALIVE` 用的是内核默认（7200s 空闲 + 75s×9）。于是路径被静默丢弃（睡眠唤醒 / NAT / 路由器丢流，无 FIN/RST）时，客户端只能等内核 `tcp_retries2`（约 15 分钟，且有未确认数据时才触发），界面全程冻结。

修复落在内核：`crates/minishell-ssh/src/lib.rs` 新增 `configure_liveness()`，`create_session()` 调用它；`run_session_loop()` 改用 `flush_pending()` 缓冲 stdin、keepalive 错误视为断开；`connect()` 的重连循环在 `prepare_session` 失败时不再退出。验证：`cargo test -p minishell-ssh` 18 项全绿（含 4 个新回归测试），`cargo build --release` 通过。

## 未覆盖的 seam（已知）

真正的黑洞对端无法在进程内构造——需要网络路径丢包（root/iptables、netns+netfilter，或真实睡眠）。因此**端到端症状没有自动化回归测试**：现有测试锁的是「socket 选项确实生效」与「非阻塞写入不丢字节」两半，外加 libssh2 源码层面的语义核实。人工验证步骤见 PR 描述 / Comments。

## Comments

人工验证（需要真机睡眠，agent 无法自动执行）：

1. `cargo build --release && cargo run --release -- <某台 Linux 机器的 ip>`，登录成功后停在 shell。
2. 合上笔记本（或 `systemctl suspend`）触发睡眠；**唤醒后不要碰键盘**。
3. 预期：≤ 60 秒内出现 `Connection lost. Press any key to reconnect... (attempt 1/3)`，而不是无限期卡在 shell。旧版本是永远不出现（或约 15 分钟后才出现）。
4. 按一次**回车**：预期立即重连成功（唤醒后路由未就绪时，第一次失败也会打印 `Reconnect failed: ...` 并再次提示，而不是退出程序）。
5. 重复 3 次仍失败才退出，并在错误里带上最后一次原因。

路由器（device=Router）验证同一流程：路由器丢流通常完全无 FIN/RST，是修复前最容易永久卡死的场景。

判定修复生效的最短路径：唤醒后对着卡住的 shell 按几下回车完全无回显 → 现在应在 1 分钟内变成重连提示。

