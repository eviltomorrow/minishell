# minishell

一个 SSH 机器管理终端工具（TUI）。管理一批"机器"的登录信息，支持快速搜索、SSH 登录、Excel 导入导出，并在启动时探测每台机器是否可达。

## Language

**机器 (Machine)**:
一条持久化的 SSH 目标配置：IP、NAT-IP、端口、用户名、认证方式、设备、备注。入库持久化的配置。
_Avoid_: 主机、服务器、host

**NAT-IP**:
机器在 NAT 网络下的对外地址。非空且非 `-` 时，登录目标使用它而非 IP。
_Avoid_: 外网 IP

**登录目标 (effective host)**:
实际用于 SSH 登录和探测的端点——有有效 NAT-IP 时用 NAT-IP，否则用 IP，端口用机器端口。
_Avoid_: 连接地址、目标机

**探测 (probe)**:
启动时对每台机器的登录目标端口做一次 TCP 可达性检查，不认证、与登录无关。
_Avoid_: ping、在线检查、健康检查

**探测结果 (probe result)**:
一次探测产生的内存瞬态快照：OK / DOWN / UNKNOWN，可选附延迟毫秒数。瞬态、不持久化，与持久化的机器配置相区分。
_Avoid_: 状态（作为正式术语）

**OK / DOWN / UNKNOWN**:
探测结果的三个可能状态。OK 表示端口 TCP 可达；DOWN 表示不可达或超时；UNKNOWN 表示尚未探测。用三档而非两档——"未探测"和"不可达"本质不同。
_Avoid_: 在线、离线（含义偏向 ICMP）
