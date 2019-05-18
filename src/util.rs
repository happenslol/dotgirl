use crate::{Result, Error};
use std::{fs, path::{Path, PathBuf}};

pub fn is_symlink<P: AsRef<Path>>(path: P) -> Result<bool> {
    let result = fs::symlink_metadata(path)?
        .file_type()
        .is_symlink();

    Ok(result)
}

pub fn get_name(path: &PathBuf) -> Result<String> {
    let result = path
        .components()
        .last()
        .ok_or(Error::LastComponentInvalid(
            String::from(path.to_str().unwrap_or(""))
        ))?
        .as_os_str()
        .to_str()
        .ok_or(Error::LastComponentInvalid(
            String::from(path.to_str().unwrap_or(""))
        ))?
        .trim_start_matches(".")
        .to_owned();

    Ok(result)
}

pub fn dir_contents_equal() -> Result<bool> {
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_name_should_work() {
        // should return directory names
        let dir_path = PathBuf::from("/foo/bar/baz/");
        let dir_name = get_name(&dir_path).unwrap();
        assert_eq!(dir_name, "baz".to_owned());

        // should return file names
        let file_path = PathBuf::from("/foo/bar/baz.conf");
        let file_name = get_name(&file_path).unwrap();
        assert_eq!(file_name, "baz.conf".to_owned());

        // should trim dots at the start
        let dot_path = PathBuf::from("/foo/bar/.baz.conf");
        let dot_name = get_name(&dot_path).unwrap();
        assert_eq!(dot_name, "baz.conf".to_owned());
    }
}

