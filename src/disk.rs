use crate::Result;
use std::{path::{Path, PathBuf}, fs::File};

#[cfg(not(test))]
pub type Disk = os::OsFilesystem;

#[cfg(test)]
pub type Disk = memory::MemoryFilesystem;

pub trait Filesystem {
    fn get<P: AsRef<Path>>(path: P) -> Result<String>;
    fn put<P: AsRef<Path>>(path: P, content: &str) -> Result<()>;

    fn mkdir_all<P: AsRef<Path>>(path: P) -> Result<()>;
    fn remove<P: AsRef<Path>>(path: P) -> Result<()>;
    fn copy<T: AsRef<Path>, U: AsRef<Path>>(from: T, to: U) -> Result<()>;
    fn symlink<T: AsRef<Path>, U: AsRef<Path>>(from: T, to: U) -> Result<()>;

    fn is_dir<P: AsRef<Path>>(path: P) -> bool;
    fn is_file<P: AsRef<Path>>(path: P) -> bool;
    fn is_symlink<P: AsRef<Path>>(path: P) -> bool;
}

mod os {
    use super::*;
    use std::{fs, io::prelude::*};

    #[allow(dead_code)]
    pub struct OsFilesystem;
    impl Filesystem for OsFilesystem {
        fn get<P: AsRef<Path>>(path: P) -> Result<String> {
            let contents = fs::read_to_string(&path)?;
            Ok(contents)
        }

        fn put<P: AsRef<Path>>(path: P, content: &str) -> Result<()> {
            let mut out = File::create(&path)?;
            out.write_all(content.as_bytes())?;
            Ok(())
        }

        fn mkdir_all<P: AsRef<Path>>(path: P) -> Result<()> {
            fs::create_dir_all(&path)?;
            Ok(())
        }

        fn remove<P: AsRef<Path>>(path: P) -> Result<()> {
            fs::remove_dir_all(&path)?;
            Ok(())
        }

        fn copy<T: AsRef<Path>, U: AsRef<Path>>(from: T, to: U) -> Result<()> {
            let mut options = fs_extra::dir::CopyOptions::new();
            options.copy_inside = true;
            options.overwrite = true;

            fs_extra::dir::copy(&from, &to, &options)?;
            Ok(())
        }

        fn symlink<T: AsRef<Path>, U: AsRef<Path>>(from: T, to: U) -> Result<()> {
            use std::os::unix::fs::symlink;
            symlink(&from, &to)?;
            Ok(())
        }

        fn is_dir<P: AsRef<Path>>(path: P) -> bool {
            let buf = PathBuf::from(path.as_ref());
            buf.is_dir()
        }

        fn is_file<P: AsRef<Path>>(path: P) -> bool {
            let buf = PathBuf::from(path.as_ref());
            buf.is_file()
        }

        fn is_symlink<P: AsRef<Path>>(path: P) -> bool {
            fs::symlink_metadata(path)
                .ok()
                .map(|it| it.file_type().is_symlink())
                .unwrap_or(false)
        }
    }
}

#[cfg(test)]
mod memory {
    use super::*;
    use std::{
        sync::Mutex,
        collections::HashMap,
    };

    #[derive(Clone, Debug)]
    enum Entry {
        File(Option<String>),
        Dir,
        Symlink,
    }

    lazy_static! {
        static ref DISK: Mutex<HashMap<String, Entry>> =
            Mutex::new(HashMap::new());
    }

    #[allow(dead_code)]
    pub struct MemoryFilesystem;
    impl Filesystem for MemoryFilesystem {
        fn get<P: AsRef<Path>>(path: P) -> Result<String> {
            let key = format!("{}", path.as_ref().display());
            let entry = {
                let disk = DISK.lock().unwrap();
                disk.get(&key).cloned()
            };

            if let Some(entry) = entry {
                if let Entry::File(Some(content)) = entry {
                    Ok(content.clone())
                } else {
                    Err(crate::Error::Simple("file was not readable"))
                }
            } else {
                Err(crate::Error::Simple("file not found"))
            }
        }

        fn put<P: AsRef<Path>>(path: P, content: &str) -> Result<()> {
            let key = format!("{}", path.as_ref().display());
            let content = String::from(content);
            DISK.lock().unwrap().insert(key, Entry::File(Some(content)));
            Ok(())
        }

        fn mkdir_all<P: AsRef<Path>>(path: P) -> Result<()> {
            let parts = PathBuf::from(path.as_ref());
            let mut buf = PathBuf::from("");
            let mut disk = DISK.lock().unwrap();

            for part in parts.components() {
                buf.push(&part);
                let key = format!("{}", buf.display());

                match disk.get(&key) {
                    Some(Entry::File(_)) => return Err(crate::Error::Simple("file existed")),
                    Some(Entry::Symlink) => return Err(crate::Error::Simple("symlink existed")),
                    _ => {},
                };

                disk.insert(key, Entry::Dir);
            }

            Ok(())
        }

        fn remove<P: AsRef<Path>>(path: P) -> Result<()> {
            let mut disk = DISK.lock().unwrap();
            let key = format!("{}", path.as_ref().display());
            let to_delete = disk
                .iter()
                .filter(|(k, _)| k.starts_with(&key))
                .map(|(k, _)| k.clone())
                .collect::<Vec<String>>();

            to_delete.iter().for_each(|it| {
                disk.remove(it);
            });

            Ok(())
        }

        fn copy<T: AsRef<Path>, U: AsRef<Path>>(from: T, to: U) -> Result<()> {
            let mut disk = DISK.lock().unwrap();
            let from_key = format!("{}", from.as_ref().display());
            let from_entry = disk
                .get(&from_key)
                .ok_or(crate::Error::Simple("copy src didn't exist"))?
                .clone();

            let key = format!("{}", to.as_ref().display());
            disk.insert(key, from_entry);

            Ok(())
        }

        fn symlink<T: AsRef<Path>, U: AsRef<Path>>(_: T, to: U) -> Result<()> {
            let mut disk = DISK.lock().unwrap();
            let key = format!("{}", to.as_ref().display());
            disk.insert(key, Entry::Symlink);
            Ok(())
        }

        fn is_dir<P: AsRef<Path>>(path: P) -> bool {
            let disk = DISK.lock().unwrap();
            let key = format!("{}", path.as_ref().display());
            if let Some(Entry::Dir) = disk.get(&key) {
                true
            } else {
                false
            }
        }

        fn is_file<P: AsRef<Path>>(path: P) -> bool {
            let disk = DISK.lock().unwrap();
            let key = format!("{}", path.as_ref().display());
            if let Some(Entry::File(_)) = disk.get(&key) {
                true
            } else {
                false
            }
        }

        fn is_symlink<P: AsRef<Path>>(path: P) -> bool {
            let disk = DISK.lock().unwrap();
            let key = format!("{}", path.as_ref().display());
            if let Some(Entry::Symlink) = disk.get(&key) {
                true
            } else {
                false
            }
        }
    }
}

