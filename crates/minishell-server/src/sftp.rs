use russh_sftp::server::Handler;
use russh_sftp::protocol::{Status, StatusCode, FileAttributes, OpenFlags, Handle, Attrs, Name, Data, Version, File};
use russh_sftp::server::StatusReply;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use tokio::fs as async_fs;
use std::os::unix::fs::PermissionsExt;

#[derive(Debug)]
pub struct SftpHandler {
    root: PathBuf,
    open_dirs: HashMap<String, Vec<File>>,
    open_files: HashMap<String, async_fs::File>,
    next_handle: u64,
}

impl SftpHandler {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            open_dirs: HashMap::new(),
            open_files: HashMap::new(),
            next_handle: 0,
        }
    }

    fn next_handle(&mut self) -> String {
        self.next_handle += 1;
        format!("h{}", self.next_handle)
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    fn file_attrs(metadata: &fs::Metadata) -> FileAttributes {
        let perm = metadata.permissions().mode();
        let size = metadata.len();
        let mtime = metadata.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);
        let atime = metadata.accessed()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);

        FileAttributes {
            size: Some(size),
            uid: Some(unsafe { libc::getuid() }),
            gid: Some(unsafe { libc::getgid() }),
            permissions: Some(perm),
            atime: Some(atime),
            mtime: Some(mtime),
            user: None,
            group: None,
        }
    }

    fn make_file(name: String, metadata: &fs::Metadata) -> File {
        File {
            filename: name,
            longname: String::new(),
            attrs: Self::file_attrs(metadata),
        }
    }
}

impl Handler for SftpHandler {
    type Error = StatusReply;

    fn unimplemented(&self) -> Self::Error {
        StatusReply::from(StatusCode::OpUnsupported)
    }

    async fn init(&mut self, _version: u32, _extensions: HashMap<String, String>) -> Result<Version, Self::Error> {
        Ok(Version {
            version: 3,
            extensions: HashMap::new(),
        })
    }

    async fn open(&mut self, id: u32, filename: String, pflags: OpenFlags, _attrs: FileAttributes) -> Result<Handle, Self::Error> {
        let path = self.resolve_path(&filename);
        let handle = self.next_handle();

        let file = if pflags.contains(OpenFlags::CREATE) || pflags.contains(OpenFlags::TRUNCATE) {
            async_fs::File::create(&path).await
        } else {
            async_fs::File::open(&path).await
        };

        match file {
            Ok(f) => {
                self.open_files.insert(handle.clone(), f);
                Ok(Handle { id, handle })
            }
            Err(_) => Err(StatusReply::from(StatusCode::NoSuchFile))
        }
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.open_files.remove(&handle);
        self.open_dirs.remove(&handle);
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, Self::Error> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let file = self.open_files.get_mut(&handle).ok_or_else(|| StatusReply::from(StatusCode::NoSuchFile))?;

        file.seek(std::io::SeekFrom::Start(offset)).await.map_err(|_| StatusReply::from(StatusCode::Failure))?;

        let mut buf = vec![0u8; len as usize];
        let n = file.read(&mut buf).await.map_err(|_| StatusReply::from(StatusCode::Failure))?;
        buf.truncate(n);

        Ok(Data { id, data: buf })
    }

    async fn write(&mut self, id: u32, handle: String, offset: u64, data: Vec<u8>) -> Result<Status, Self::Error> {
        use tokio::io::{AsyncWriteExt, AsyncSeekExt};

        let file = self.open_files.get_mut(&handle).ok_or_else(|| StatusReply::from(StatusCode::NoSuchFile))?;

        file.seek(std::io::SeekFrom::Start(offset)).await.map_err(|_| StatusReply::from(StatusCode::Failure))?;

        file.write_all(&data).await.map_err(|_| StatusReply::from(StatusCode::Failure))?;

        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        let dir_path = self.resolve_path(&path);
        let handle = self.next_handle();

        let entries = fs::read_dir(&dir_path).map_err(|_| StatusReply::from(StatusCode::NoSuchFile))?;

        let mut items = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|_| StatusReply::from(StatusCode::Failure))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "." || name == ".." { continue; }
            let metadata = entry.metadata().map_err(|_| StatusReply::from(StatusCode::Failure))?;
            items.push(Self::make_file(name, &metadata));
        }

        self.open_dirs.insert(handle.clone(), items);
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        let entries = self.open_dirs.get_mut(&handle).ok_or_else(|| StatusReply::from(StatusCode::NoSuchFile))?;

        match entries.pop() {
            Some(file) => Ok(Name {
                id,
                files: vec![file],
            }),
            None => Err(StatusReply::from(StatusCode::Eof))
        }
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let full_path = self.resolve_path(&path);
        let metadata = fs::symlink_metadata(&full_path).map_err(|_| StatusReply::from(StatusCode::NoSuchFile))?;
        Ok(Attrs { id, attrs: Self::file_attrs(&metadata) })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        let full_path = self.resolve_path(&path);
        let metadata = fs::metadata(&full_path).map_err(|_| StatusReply::from(StatusCode::NoSuchFile))?;
        Ok(Attrs { id, attrs: Self::file_attrs(&metadata) })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        let file = self.open_files.get(&handle).ok_or_else(|| StatusReply::from(StatusCode::NoSuchFile))?;
        let metadata = file.metadata().await.map_err(|_| StatusReply::from(StatusCode::Failure))?;
        Ok(Attrs { id, attrs: Self::file_attrs(&metadata) })
    }

    async fn setstat(&mut self, id: u32, path: String, attrs: FileAttributes) -> Result<Status, Self::Error> {
        let full_path = self.resolve_path(&path);
        if let Some(perm) = attrs.permissions {
            fs::set_permissions(&full_path, fs::Permissions::from_mode(perm)).map_err(|_| StatusReply::from(StatusCode::Failure))?;
        }
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn fsetstat(&mut self, id: u32, handle: String, attrs: FileAttributes) -> Result<Status, Self::Error> {
        let file = self.open_files.get(&handle).ok_or_else(|| StatusReply::from(StatusCode::NoSuchFile))?;
        if let Some(perm) = attrs.permissions {
            file.set_permissions(fs::Permissions::from_mode(perm)).await.map_err(|_| StatusReply::from(StatusCode::Failure))?;
        }
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn mkdir(&mut self, id: u32, path: String, _attrs: FileAttributes) -> Result<Status, Self::Error> {
        let full_path = self.resolve_path(&path);
        fs::create_dir(&full_path).map_err(|_| StatusReply::from(StatusCode::Failure))?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        let full_path = self.resolve_path(&path);
        fs::remove_dir(&full_path).map_err(|_| StatusReply::from(StatusCode::Failure))?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        let full_path = self.resolve_path(&filename);
        fs::remove_file(&full_path).map_err(|_| StatusReply::from(StatusCode::Failure))?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn rename(&mut self, id: u32, oldpath: String, newpath: String) -> Result<Status, Self::Error> {
        let old = self.resolve_path(&oldpath);
        let new = self.resolve_path(&newpath);
        fs::rename(&old, &new).map_err(|_| StatusReply::from(StatusCode::Failure))?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let full_path = self.resolve_path(&path);
        let canonical = fs::canonicalize(&full_path).map_err(|_| StatusReply::from(StatusCode::NoSuchFile))?;
        Ok(Name {
            id,
            files: vec![File {
                filename: canonical.to_string_lossy().to_string(),
                longname: String::new(),
                attrs: FileAttributes::default(),
            }],
        })
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        let full_path = self.resolve_path(&path);
        let target = fs::read_link(&full_path).map_err(|_| StatusReply::from(StatusCode::NoSuchFile))?;
        Ok(Name {
            id,
            files: vec![File {
                filename: target.to_string_lossy().to_string(),
                longname: String::new(),
                attrs: FileAttributes::default(),
            }],
        })
    }

    async fn symlink(&mut self, id: u32, linkpath: String, targetpath: String) -> Result<Status, Self::Error> {
        let link = self.resolve_path(&linkpath);
        let target = self.resolve_path(&targetpath);
        std::os::unix::fs::symlink(&target, &link).map_err(|_| StatusReply::from(StatusCode::Failure))?;
        Ok(Status {
            id,
            status_code: StatusCode::Ok,
            error_message: "".to_string(),
            language_tag: "".to_string(),
        })
    }
}
