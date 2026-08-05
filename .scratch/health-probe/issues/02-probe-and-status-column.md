# 02 — 启动探测 + 状态列全链路

**What to build:** TUI 启动时自动触发一次探测（不冻结界面），结果自动刷新显示。主循环由阻塞读事件改为 `poll(50ms)`，每轮收取探测通道里的新结果；列表最右新增常驻状态列，按结果着色（OK 绿 / DOWN 红 / UNKNOWN 灰）并显示延迟毫秒数。探测进行中与探测完成均无需用户按键干预。

**Blocked by:** 01 — 探测引擎与状态建模

**Status:** resolved

- [x] TUI 启动即对全部机器触发探测，界面全程可交互、无冻结
- [x] 主循环改为带短超时的轮询（如 `poll(50ms)`），每轮收取探测结果，新结果到达后自动重绘
- [x] 列表最右新增常驻状态列：OK 绿 / DOWN 红 / UNKNOWN 灰，单元格显示延迟毫秒数
- [x] 探测完成后无需按键即可看到全部状态就位

## Answer

`table.rs`：`MachineTable` 新增 `status_col` + `status_styles`（逐行独立着色），渲染时状态列颜色叠加于行样式之上；`status_cell()` 格式化为 `● Nms`（绿）/ `● down`（红）/ `·`（灰）；`default_columns`/`secrets_columns` 各追加一列空的、固定宽度状态列。`app.rs`：`AppState` 加 `status: HashMap<i64,ProbeResult>` + `probe: Option<ProbeHandle>`；启动即 `start_probe()`；主循环改 `poll(50ms)`，每轮 `poll_probe()` 收取结果、有更新即 `rebuild_table()`。`cargo build --release` 与全量测试通过。
