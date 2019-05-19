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
            let buf = PathBuf::from(path.as_ref());
            if buf.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }

            Ok(())
        }

        fn copy<T: AsRef<Path>, U: AsRef<Path>>(from: T, to: U) -> Result<()> {
            let buf = PathBuf::from(from.as_ref());

            if buf.is_dir() {
                let mut options = fs_extra::dir::CopyOptions::new();
                options.copy_inside = true;
                options.overwrite = true;

                fs_extra::dir::copy(&from, &to, &options)?;
            } else {
                fs::copy(&from, &to)?;
            }

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
        cell::RefCell,
        collections::HashMap,
    };

    #[derive(Clone, Debug)]
    enum Entry {
        File(Option<String>),
        Dir,
        Symlink,
    }

    // each thread needs its own in-memory filesystem, since tests will run in parallel
    // and conflict if we don't separate their filesystems.
    thread_local! {
        static DISK: RefCell<HashMap<String, Entry>> = RefCell::new(HashMap::new());
    }

    #[allow(dead_code)]
    pub struct MemoryFilesystem;

    impl MemoryFilesystem {
        pub fn print() {
            DISK.with(|disk| {
                let disk = disk.borrow();
                let mut keys = disk.keys().map(|it| it.clone()).collect::<Vec<String>>();
                keys.sort_unstable();

                for k in keys.iter() {
                    println!("{} -> {:?}", k, disk[k]);
                }
            });
        }

        pub fn clear() {
            DISK.with(|disk| {
                disk.borrow_mut().clear();
            });
        }
    }

    impl Filesystem for MemoryFilesystem {
        fn get<P: AsRef<Path>>(path: P) -> Result<String> {
            let mut result = Ok(String::from(""));

            DISK.with(|disk| {
                let disk = disk.borrow();

                let key = format!("{}", path.as_ref().display());
                let entry = disk.get(&key).cloned();

                if let Some(entry) = entry {
                    if let Entry::File(Some(content)) = entry {
                        result = Ok(content.clone())
                    } else {
                        result = Err(crate::Error::Simple("file was not readable"))
                    }
                } else {
                    result = Err(crate::Error::Simple("file not found"))
                }
            });

            result
        }

        fn put<P: AsRef<Path>>(path: P, content: &str) -> Result<()> {
            let key = format!("{}", path.as_ref().display());
            let content = String::from(content);
            DISK.with(|disk| {
                disk.borrow_mut().insert(key, Entry::File(Some(content)));
            });

            Ok(())
        }

        fn mkdir_all<P: AsRef<Path>>(path: P) -> Result<()> {
            let mut result = Ok(());

            DISK.with(|disk| {
                let mut disk = disk.borrow_mut();

                let parts = PathBuf::from(path.as_ref());
                let mut buf = PathBuf::from("");

                for part in parts.components() {
                    buf.push(&part);
                    let key = format!("{}", buf.display());

                    match disk.get(&key) {
                        Some(Entry::File(_)) => {
                            result = Err(crate::Error::Simple("file existed"));
                        },
                        Some(Entry::Symlink) => {
                            result = Err(crate::Error::Simple("symlink existed"));
                        },
                        _ => {},
                    };

                    disk.insert(key, Entry::Dir);
                }
            });

            result
        }

        fn remove<P: AsRef<Path>>(path: P) -> Result<()> {
            DISK.with(|disk| {
                let mut disk = disk.borrow_mut();

                let key = format!("{}", path.as_ref().display());
                let to_delete = disk
                    .iter()
                    .filter(|(k, _)| k.starts_with(&key))
                    .map(|(k, _)| k.clone())
                    .collect::<Vec<String>>();

                to_delete.iter().for_each(|it| {
                    disk.remove(it);
                });
            });

            Ok(())
        }

        fn copy<T: AsRef<Path>, U: AsRef<Path>>(from: T, to: U) -> Result<()> {
            let mut result = Ok(());

            DISK.with(|disk| {
                let mut disk = disk.borrow_mut();

                let from_key = format!("{}", from.as_ref().display());
                let from_entry = disk
                    .get(&from_key)
                    .cloned()
                    .ok_or(crate::Error::Simple("copy src didn't exist"));

                let from_entry = if let Err(error) = from_entry {
                    result = Err(error);
                    return;
                } else {
                    from_entry.unwrap()
                };

                let key = format!("{}", to.as_ref().display());

                if let Entry::Dir = from_entry {
                    let to_save = disk
                        .keys()
                        .filter(|it| it.starts_with(&from_key))
                        .map(|it| {
                            let suffix = it.trim_start_matches(&from_key);
                            (it.clone(), format!("{}{}", key, suffix))
                        })
                        .collect::<Vec<(String, String)>>();

                    to_save.into_iter().for_each(|(from, to)| {
                        let entry = disk[&from].clone();
                        disk.insert(to, entry);
                    });
                }

                disk.insert(key, from_entry);
            });
            Ok(())
        }

        fn symlink<T: AsRef<Path>, U: AsRef<Path>>(_: T, to: U) -> Result<()> {
            DISK.with(|disk| {
                let mut disk = disk.borrow_mut();
                let key = format!("{}", to.as_ref().display());
                disk.insert(key, Entry::Symlink);
            });

            Ok(())
        }

        fn is_dir<P: AsRef<Path>>(path: P) -> bool {
            let mut result = false;

            DISK.with(|disk| {
                let disk = disk.borrow();
                let key = format!("{}", path.as_ref().display());
                if let Some(Entry::Dir) = disk.get(&key) {
                    result = true;
                }
            });

            result
        }

        fn is_file<P: AsRef<Path>>(path: P) -> bool {
            let mut result = false;

            DISK.with(|disk| {
                let disk = disk.borrow();
                let key = format!("{}", path.as_ref().display());
                if let Some(Entry::File(_)) = disk.get(&key) {
                    result = true;
                }
            });

            result
        }

        fn is_symlink<P: AsRef<Path>>(path: P) -> bool {
            let mut result = false;

            DISK.with(|disk| {
                let disk = disk.borrow();
                let key = format!("{}", path.as_ref().display());
                if let Some(Entry::Symlink) = disk.get(&key) {
                    result = true;
                }
            });

            result
        }
    }
}

