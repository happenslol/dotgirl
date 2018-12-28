use std::{
    io::Write,
    fs::{self, File},
    path::{Path, PathBuf}
};
use clap::{clap_app, AppSettings};
use serde::{Serialize, Deserialize};

#[derive(Debug)]
enum Error {
    IoError(std::io::Error),
    IoExtraError(fs_extra::error::Error),
    HomedirNotFoundError,
    ParseError(ron::de::Error),
    SerializeError(ron::ser::Error),
    LastComponentInvalid(String),
}

impl std::convert::From<fs_extra::error::Error> for Error {
    fn from(error: fs_extra::error::Error) -> Self {
        Error::IoExtraError(error)
    }
}

impl std::convert::From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Error::IoError(error)
    }
}

impl std::convert::From<ron::de::Error> for Error {
    fn from(error: ron::de::Error) -> Self {
        Error::ParseError(error)
    }
}

impl std::convert::From<ron::ser::Error> for Error {
    fn from(error: ron::ser::Error) -> Self {
        Error::SerializeError(error)
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Serialize, Deserialize)]
struct Bundle {
    id: String,
    author: String,
    entries: Vec<Entry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    src: PathBuf,
    dst: PathBuf,
}

type Lockfile = Vec<Bundle>;

fn main() -> Result<()> {
    let matches = clap_app!(dotgirl =>
        (version: env!("CARGO_PKG_VERSION"))
        (author: env!("CARGO_PKG_AUTHORS"))
        (about: env!("CARGO_PKG_DESCRIPTION"))
        (@subcommand add =>
            (about: "add to a bundle")
            (@arg BUNDLE: +required "bundle name")
            (@arg INPUT: +required ... "input")
        )
    )
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .get_matches();

    match matches.subcommand() {
        ("add", Some(matches)) => {
            let bundle = matches.value_of("BUNDLE")
                .expect("Invalid: BUNDLE is required");

            let paths = matches.values_of("INPUT")
                .expect("Invalid: INPUT is required")
                .map(Path::new)
                .map(|it| it.canonicalize().expect("invalid path"))
                .collect::<Vec<PathBuf>>();

			add(&bundle, &paths)?;
        },
        _ => {},
    };

	Ok(())
}

fn get_dir() -> Result<PathBuf> {
    let mut path = dirs::home_dir().ok_or(Error::HomedirNotFoundError)?;
    path.push("dotgirl");
    fs::create_dir_all(&path)?;

    Ok(path)
}

fn get_lockfile() -> Result<Lockfile> {
    let mut path = dirs::home_dir().ok_or(Error::HomedirNotFoundError)?;
    path.push("dotgirl");

    if !path.is_dir() {
        return Ok(vec![]);
    }

    path.push("lock.ron");

    if !path.is_file() {
        return Ok(vec![]);
    }

    let contents = fs::read_to_string(path)?;
    let parsed = ron::de::from_str::<Lockfile>(&contents)?;

    Ok(parsed)
}

fn get_bundle_dir(name: &str) -> Result<PathBuf> {
    let mut path = dirs::home_dir().ok_or(Error::HomedirNotFoundError)?;
    path.push("dotgirl");
    path.push(name);
    fs::create_dir_all(&path)?;

    Ok(path)
}

fn add(bundle: &str, paths: &Vec<PathBuf>) -> Result<()> {
    let mut lockfile = get_lockfile()?;

    // TODO(happens): validate paths
    let dir = get_bundle_dir(bundle)?;
    let entries = paths
        .iter()
        .map(|src| {
            let src = src.clone();
            let mut dst = dir.clone();
            let dst_last = src
                .components()
                .last()
                .ok_or(Error::LastComponentInvalid(
                    String::from(src.to_str().unwrap_or(""))
                ))?
                .as_os_str()
                .to_str()
                .ok_or(Error::LastComponentInvalid(
                    String::from(src.to_str().unwrap_or(""))
                ))?;

            let mut dst_last = String::from(dst_last);
            if dst_last.starts_with(".") {
                dst_last.remove(0);
            }

            dst.push(dst_last);

            if src.is_dir() {
                let mut options = fs_extra::dir::CopyOptions::new();
                options.copy_inside = true;
                options.overwrite = true;
                fs::create_dir_all(&dst)?;
                fs_extra::dir::copy(&src, &dst, &options)?;
            } else if src.is_file() {
                fs::copy(&src, &dst)?;
            } else {
                println!("Does not exist: {:?}", src);
            }

            Ok(Entry { src, dst })
        })
        .map(|it| {
            if it.is_err() {
                println!("err while adding: {:?}", it);
            }

            it
        })
        .filter_map(Result::ok)
        .collect::<Vec<Entry>>();

    let bundle = Bundle {
        id: String::from(bundle),
        author: String::from("me"),
        entries,
    };

    link(&bundle);
    lockfile.push(bundle);

    let mut lock_path = get_dir()?;
    lock_path.push("lock.ron");

    let updated = ron::ser::to_string(&lockfile)?;
    let mut file = File::create(lock_path)?;
    file.write_all(updated.as_bytes())?;

    Ok(())
}

fn link(bundle: &Bundle) {
}

