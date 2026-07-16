# CLI Push / Pull — Quick File Transfer

## Overview

Add `minishell push` and `minishell pull` CLI subcommands for fast file
transfers between local and remote machines, with SCP-compatible semantics,
recursive directory support, progress output, and permission preservation.

## CLI Interface

```
minishell push   <query> <local_path> <remote_path>  [-r]
minishell pull   <query> <remote_path> <local_path>  [-r]
```

- `<query>` — IP / remark / ID 匹配（复用现有 quick-login 逻辑）
  - 唯一匹配 → 直接连接传输
  - 多台匹配 → 弹出 Selector 让用户选择
  - 无匹配 → 报错退出
- `-r` — 递归模式，传输目录（SCP 标志，不加则目录报错）

## Path Semantics (SCP-aligned)

### push

| local | remote 已存在 | `-r` | 行为 |
|---|---|---|---|
| 文件 | - | 任意 | 单文件上传。remote 尾 `/` → 作为目录，否则最后一节为文件名 |
| 目录 | - | 要 | 递归上传，在远端新建目录 |
| 目录 | 目录 | 要 | 内容传入该远端目录 |
| 目录 | 文件 | 要 | **报错退出**，类型冲突 |
| 目录 | 任意 | 无 | **报错退出**，提示加 `-r` |

### pull

| remote | local 已存在 | `-r` | 行为 |
|---|---|---|---|
| 文件 | - | 任意 | 单文件下载 |
| 目录 | - | 要 | 递归下载到本地 |
| 目录 | 目录 | 要 | 内容传入本地目录 |
| 目录 | 文件 | 要 | **报错退出**，类型冲突 |

## Progress Output

每文件一行，`\r` 覆盖更新，风格类似 `scp -v`：

```
file.txt              100%   45MB   4.5MB/s   00:10
src/main.rs            42%  1.2MB   2.1MB/s   00:01
```

递归模式下目录名作前缀。传输完成后打印汇总：

```
Transferred: 12 files, 128.5MB  (0 errors)
```

如有错误（单文件失败不阻断整个传输）：

```
Transferred: 11/12 files, 120.0MB  (1 error)
  ✗  secrets/key.pem  —  Permission denied
```

## Permission Preservation

### Upload
- 读取本地文件权限位（`mode & 0o777`）
- 传输完成后 `sftp.setstat()` 设置远程文件权限
- 目录传输时：`mkdir` 后立即设置目录权限
- 失败时不阻断传输，但输出 warning

### Download
- 读取远程 `FileStat.perm`
- 传输完成后 `std::os::unix::fs::PermissionsExt::from_mode()` 设置本地权限
- 目录传输时：`create_dir` 后立即设置

## Recursive Implementation

### Upload
```
push_recursive(sftp, local_root, remote_root, progress):
  for entry in fs::read_dir(local_root):
    if entry is dir and -r:
      new_remote = remote_root / entry.name
      sftp.mkdir(new_remote, 0o755)
      set_perm(new_remote, entry.mode)
      push_recursive(sftp, entry.path, new_remote, progress)
    elif entry is file:
      upload_file(sftp, entry.path, remote_root / entry.name, progress)
      set_perm(remote_root / entry.name, entry.mode)
```

### Download
```
pull_recursive(sftp, remote_root, local_root, progress):
  for entry in sftp.readdir(remote_root):
    if entry is dir and -r:
      new_local = local_root / entry.name
      fs::create_dir(new_local)
      set_perm(new_local, entry.perm)
      pull_recursive(sftp, entry.path, new_local, progress)
    elif entry is file:
      download_file(sftp, entry.path, local_root / entry.name, progress)
      set_perm(local_root / entry.name, entry.perm)
```

## File Structure Changes

### `minishell-ssh/src/sftp.rs`
- `upload_recursive(sftp, local, remote, progress_cb)` — 递归上传
- `download_recursive(sftp, remote, local, progress_cb)` — 递归下载
- `mkdir_p(sftp, path)` — 递归创建远程目录
- `set_perm_remote(sftp, path, mode)` — 设置远程文件/目录权限
- `set_perm_local(path, mode)` — 设置本地文件/目录权限

### `minishell-cli/src/main.rs`
- 新增 `Commands::Push` / `Commands::Pull` 子命令
- 参数解析：query, local_path, remote_path, recursive_flag
- 复用 `db_path()` / `open_db()` / 机器匹配逻辑
- 终端进度输出（`\r` 覆盖行 + 最终汇总）

### Progress Callback Type

```rust
pub struct TransferProgress {
    pub file_name: String,      // 当前文件名（含相对路径）
    pub bytes_written: u64,
    pub total_bytes: u64,
    pub file_index: usize,      // 当前文件序号
    pub total_files: usize,
}
```

## Dependencies

- `minishell-ssh` 新增依赖：无（已有 `ssh2` + `anyhow`）
- `minishell-cli` 新增依赖：无（已有 `clap` + `minishell-ssh`）

## Error Handling

- 单文件传输失败 → 记录错误，继续下一个文件
- 最终汇总报告成功/失败数量
- 连接失败 → 立即报错退出（不尝试传输）
- 权限设置失败 → 记录 warning，不阻断
- Windows 平台：权限设置跳过（`#[cfg(unix)]` 保护）
