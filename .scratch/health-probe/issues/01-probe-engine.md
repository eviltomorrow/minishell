# 01 — 探测引擎与状态建模

**What to build:** 一个可编程调用的探测引擎：输入机器列表，对有界线程池并发地对每台机器的登录目标端口做 TCP 连接（每台限时），输出每台机器的探测结果（OK / DOWN / UNKNOWN，可选附延迟毫秒数）。探测结果是一次性的内存快照，与持久化的机器数据分开存放、不写库。为 TUI 提供触发入口与结果通道，供后续 UI 消费。

**Blocked by:** None — can start immediately.

**Status:** resolved

- [x] 探测引擎可输入机器列表，并发探测所有机器的登录目标（登录目标 = effective_host:port）
- [x] 每台机器有超时机制（如 2 秒），超时视为 DOWN，探测全程不阻塞调用方
- [x] 输出结果区分三态：OK（端口可达）、DOWN（不可达或超时）、UNKNOWN（尚未探测），OK 附带延迟毫秒数
- [x] 单元测试覆盖：三态判定、超时路径、并发下结果与机器一一对应

## Answer

`minishell-ssh` 新增 `probe` 模块（`crates/minishell-ssh/src/probe.rs`）：`ProbeStatus`（Ok/Down/Unknown）、`ProbeResult`（状态+延迟）、`probe_host()`（`connect_timeout`，2 秒默认超时）、`start_probe()`（有界线程池 + mpsc 通道，`ProbeHandle::try_recv()/is_done()/join()`）。5 个单元测试覆盖三态、超时、并发一一对应，全绿。
