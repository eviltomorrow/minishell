# minishell

SSH 机器管理 TUI 工具。通过终端界面集中管理服务器信息，支持快速搜索、SSH 登录、Excel 导入导出。

## 安装

```bash
cargo build --release
```

编译产物位于 `target/release/minishell`。

## 快速开始

### 启动 TUI

```bash
minishell
```

启动后进入交互界面，显示所有已保存的机器列表。

### 快速登录

```bash
minishell 10.0.0.1       # 按 IP 搜索
minishell web-server     # 按备注搜索
minishell 1              # 按 ID 登录
```

匹配到单台机器时直接 SSH 登录，匹配多台时弹出选择器。

## TUI 快捷键

| 按键      | 功能                                             |
| --------- | ------------------------------------------------ |
| `↑` / `k` | 上移光标                                         |
| `↓` / `j` | 下移光标                                         |
| `PgUp`    | 上移 10 行                                       |
| `PgDn`    | 下移 10 行                                       |
| `g`       | 跳到列表顶部                                     |
| `G`       | 跳到列表底部                                     |
| `Enter`   | 登录选中机器                                     |
| `/`       | 搜索过滤（输入即过滤，`Enter` 确认，`Esc` 清空） |
| `a`       | 添加新机器                                       |
| `e`       | 编辑选中机器                                     |
| `d`       | 删除选中机器                                     |
| `s`       | 显示/隐藏密码和密钥                              |
| `q`       | 退出                                             |

### 搜索

按 `/` 进入搜索模式，输入关键字实时过滤列表（匹配 IP 和备注）。按 `↑`/`↓` 可直接在搜索结果中导航。

### 编辑表单

- `↑`/`↓` 切换字段
- `Enter` 在最后一个字段保存
- `Esc` 取消

## 子命令

### 生成导入模板

```bash
minishell tpl                    # 输出到 minishell 同目录
minishell tpl /path/to/tpl.xlsx  # 指定路径
```

生成包含表头和示例行的 Excel 模板文件。

### 导入机器

```bash
minishell import machines.xlsx
```

从 Excel 文件导入机器数据。重复的 IP+端口 会自动跳过。

### 导出机器

```bash
minishell export                 # 输出到 minishell 同目录
minishell export /path/to/out.xlsx
```

将所有机器导出为带格式的 Excel 文件。

### 查看机器列表

```bash
minishell show
```

在终端打印所有机器的表格。

### 查看版本

```bash
minishell version
```

## Excel 模板格式

| 列              | 说明                            | 示例          |
| --------------- | ------------------------------- | ------------- |
| IP              | 服务器 IP                       | 10.0.0.1      |
| NAT-IP          | NAT 地址（可选，填 `-` 表示无） | -             |
| Port            | SSH 端口（默认 22）             | 22            |
| Username        | 登录用户                        | root          |
| Password        | 密码（可选）                    | -             |
| PrivateKey-Path | 私钥路径（可选）                | ~/.ssh/id_rsa |
| Device          | 设备标识                        | Linux         |
| Remark          | 备注                            | 生产环境      |

## 数据存储

- 数据库路径：`/tmp/minishell/db`
- SQLite 格式，单连接，自动创建目录

## SSH 连接

- 支持密码和密钥认证
- 连接超时：10 秒
- 会话最长：1 小时
- 登录/退出时显示连接信息卡片

## 项目结构

```
minishell-rust/
├── Cargo.toml                    # workspace 根
├── crates/
│   ├── minishell-core/           # Machine 模型
│   ├── minishell-store/          # SQLite 持久化
│   ├── minishell-ssh/            # SSH 连接
│   ├── minishell-tui/            # TUI 界面
│   ├── minishell-xlsx/           # Excel 导入导出
│   └── minishell-cli/            # CLI 入口
└── docs/
    └── superpowers/              # 设计文档和实施计划
```

## 依赖

| 库              | 用途          |
| --------------- | ------------- |
| ratatui         | TUI 框架      |
| crossterm       | 终端控制      |
| rusqlite        | SQLite 数据库 |
| ssh2            | SSH 连接      |
| calamine        | Excel 读取    |
| rust_xlsxwriter | Excel 写入    |
| clap            | CLI 参数解析  |

## 许可

MIT
