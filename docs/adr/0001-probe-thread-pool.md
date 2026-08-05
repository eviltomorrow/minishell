# 0001: 启动探测用有界线程池而非并入事件循环

TUI 是单线程阻塞 `event::read()` 架构，SSH 交互 I/O 刻意用 `libc::poll` 而非线程（见 AGENTS.md）。若启动探测在主线程内阻塞执行，界面会冻结。决定：用有界线程池 + mpsc 通道做一次性探测，每台机器用 `TcpStream::connect_timeout` 限时。主循环由阻塞 `event::read()` 改为 `poll(50ms)`（filebrowser 分支已有此模式），每轮 `try_recv()` 收取结果——探测完成即自动刷新，不依赖按键。

备选方案是把非阻塞 connect 并入 poll 式事件循环（轮询 stdin fd + 各探测 socket + 重绘定时器）——架构上更纯粹、保持全单线程，但需要重构 TUI 主循环，风险高。探测是启动时短暂的辅助任务、不参与交互 I/O，为它引入受限且生命周期短暂的线程可接受，也不与 SSH 会话的 poll 式 I/O 冲突。
