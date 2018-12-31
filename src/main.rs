use std::{
    io::Write,
    fs::{self, File},
    path::{Path, PathBuf}
};
use clap::{clap_app, AppSettings};
use serde::{Serialize, Deserialize};
use dialoguer::{Confirmation, Select, theme};

#[derive(Debug)]
enum Error {
    IoError(std::io::Error),
    IoExtraError(fs_extra::error::Error),
    HomedirNotFound,
    ParseError(ron::de::Error),
    SerializeError(ron::ser::Error),
    LastComponentInvalid(String),
    BundleNotFound,
    BundleMissingMeta,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        (@subcommand link =>
            (about: "link a bundle")
            (@arg BUNDLE: +required "bundle name")
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

			cmd_add(&bundle, &paths)?;
        },
        ("link", Some(matches)) => {
            let bundle = matches.value_of("BUNDLE")
                .expect("Invalid: BUNDLE is required");

            cmd_link(&bundle)?;
        },
        _ => {},
    };

	Ok(())
}

fn get_dir() -> Result<PathBuf> {
    let mut path = dirs::home_dir().ok_or(Error::HomedirNotFound)?;
    path.push("dotgirl");
    fs::create_dir_all(&path)?;

    Ok(path)
}

fn get_lockfile() -> Result<Lockfile> {
    let mut path = dirs::home_dir().ok_or(Error::HomedirNotFound)?;
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

fn write_lockfile(lockfile: &Lockfile) -> Result<()> {
    let mut lock_path = get_dir()?;
    lock_path.push("lock.ron");

    let lockfile_ser = ron::ser::to_string(&lockfile)?;
    let mut out = File::create(lock_path)?;
    out.write_all(lockfile_ser.as_bytes())?;

    Ok(())
}

fn get_bundle_dir(name: &str, create: bool) -> Result<PathBuf> {
    let mut path = dirs::home_dir().ok_or(Error::HomedirNotFound)?;
    path.push("dotgirl");
    path.push("bundle");
    path.push(name);

    if create {
        fs::create_dir_all(&path)?;
    }

    Ok(path)
}

fn get_last(path: &PathBuf) -> Result<String> {
    let last = path
        .components()
        .last()
        .ok_or(Error::LastComponentInvalid(
            String::from(path.to_str().unwrap_or(""))
        ))?
        .as_os_str()
        .to_str()
        .ok_or(Error::LastComponentInvalid(
            String::from(path.to_str().unwrap_or(""))
        ))?;

    Ok(String::from(last))
}

fn cmd_add(bundle_name: &str, paths: &Vec<PathBuf>) -> Result<()> {
    let mut lockfile = get_lockfile()?;

    let paths = paths
        .into_iter()
        .filter(|it| {
            // TODO(happens): More/better validation
            // Find out if this is a symlink, we want to skip those
            let meta = fs::symlink_metadata(&it).expect("Failed to get metadata");
            if meta.file_type().is_symlink() {
                println!("skipping {} because it is a symlink.", it.to_str().unwrap());
                return false;
            }

            true
        })
        .collect::<Vec<_>>();

    let dir = get_bundle_dir(bundle_name, true)?;
    let entries = paths
        .iter()
        .map(|dst| {
            let dst = dst.to_path_buf();

            let mut src = dir.to_path_buf();
            let src_last = get_last(&src)?;
            let mut src_last = String::from(src_last);
            if src_last.starts_with(".") {
                src_last.remove(0);
            }

            src.push(src_last);

            if dst.is_dir() {
                let mut options = fs_extra::dir::CopyOptions::new();
                options.copy_inside = false;
                options.overwrite = true;
                fs::create_dir_all(&src)?;
                fs_extra::dir::copy(&dst, &src, &options)?;
            } else if dst.is_file() {
                fs::copy(&dst, &src)?;
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
        id: String::from(bundle_name),
        author: String::from("me"),
        entries,
    };

    // Save the dotfile for the bundle itself, this has all the paths
    let dot_meta_path = dir.join("dot.ron");

    let bundle_ser = ron::ser::to_string(&bundle)?;
    let mut out = File::create(dot_meta_path)?;
    out.write_all(bundle_ser.as_bytes())?;

    // Save the new dotfile, which contains only the paths that have
    // been linked successfully (which in this case should always be
    // all of them, but still)
    let linked = link(&bundle, &[], true)?;
    lockfile.push(Bundle { entries: linked, ..bundle });
    write_lockfile(&lockfile)?;

    Ok(())
}

fn cmd_link(bundle_name: &str) -> Result<()> {
    let mut lockfile = get_lockfile()?;

    let maybe_previous = lockfile.iter().find(|it| it.id == bundle_name);
    let previous_entries = if let Some(previous) = maybe_previous {
        previous.entries
            .iter()
            .map(|it| it.dst.to_str().unwrap())
            .collect::<Vec<_>>()
    } else {
        vec![]
    };

    let dir = get_bundle_dir(bundle_name, false)?;
    if !dir.exists() || !dir.is_dir() {
        return Err(Error::BundleNotFound);
    }

    let dot_meta_path = dir.join("dot.ron");
    if !dot_meta_path.exists() || !dot_meta_path.is_file() {
        return Err(Error::BundleMissingMeta);
    }

    let contents = fs::read_to_string(dot_meta_path)?;
    let bundle = ron::de::from_str::<Bundle>(&contents)?;

    let linked = link(&bundle, &previous_entries[..], false)?;
    lockfile.retain(|it| it.id != bundle_name);
    lockfile.push(Bundle { entries: linked, ..bundle });
    write_lockfile(&lockfile)?;

    Ok(())
}

fn link(
    bundle: &Bundle,
    overwrite: &[&str],
    overwrite_all: bool
) -> Result<Vec<Entry>> {
    use std::os::unix::fs::symlink;

    let mut result = Vec::new();
    let mut overwrite_all = overwrite_all;
    for it in &bundle.entries {
        // first, make sure that all dirs leading up to the file
        // or dir exist (if there is no parent that means we
        // are placing the file at '/', which is fine
        if let Some(parent) = it.dst.parent() {
            // this is weird, but can happen
            if parent.is_file() {
                let text = format!(
                    "You're trying to link the file {}, but {} is a file. {}",
                    it.dst.display(), parent.display(),
                    "Do you want to overwrite the file and create a directory instead?",
                );

                if Confirmation::new()
                    .with_text(&text)
                    .default(false)
                    .interact()
                    .expect("Failed to show prompt")
                {
                    fs::remove_file(parent)?;
                }
            }

            if !parent.is_dir() {
                fs::create_dir_all(parent)?;
            }
        }

        if it.dst.exists() {
            if !overwrite_all && !overwrite.contains(&it.dst.to_str().unwrap()) {
                let choices = &["skip", "overwrite", "overwrite all"];
                let selection = Select::with_theme(&theme::ColorfulTheme::default())
                    .with_prompt(&format!("{} already exists.", it.dst.display()))
                    .default(0)
                    .items(&choices[..])
                    .interact()
                    .expect("Failed to show prompt");

                match selection {
                    0 => continue,
                    2 => overwrite_all = true,
                    _ => {},
                };
            }

            // if we drop through to here, we're supposed to nuke it and
            // replace it
            if it.dst.is_dir() {
                fs::remove_dir_all(&it.dst)?;
            } else if it.dst.is_file() {
                fs::remove_file(&it.dst)?;
            }
        }

        symlink(&it.src, &it.dst)?;
        result.push(it.clone());
    }

    Ok(result)
}

