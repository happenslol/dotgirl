use std::{
    io::Write,
    fs::{self, File},
    path::{Path, PathBuf}
};
use clap::{clap_app, AppSettings};
use serde::{Serialize, Deserialize};
use dialoguer::{Confirmation, Select, theme};

const STORAGE_DIR: &'static str = "dotgirl";
const BUNDLE_DIR: &'static str = "bundle";

// const CONFIG_FILE: &'static str = "config.toml";
const LOCK_FILE: &'static str = "lock.toml";
const BUNDLE_FILE: &'static str = "bundle.toml";

#[derive(Debug)]
enum Error {
    IoError(std::io::Error),
    IoExtraError(fs_extra::error::Error),
    HomedirNotFound,
    ParseError(toml::de::Error),
    SerializeError(toml::ser::Error),
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

impl std::convert::From<toml::de::Error> for Error {
    fn from(error: toml::de::Error) -> Self {
        Error::ParseError(error)
    }
}

impl std::convert::From<toml::ser::Error> for Error {
    fn from(error: toml::ser::Error) -> Self {
        Error::SerializeError(error)
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Bundle {
    id: String,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    local: String,
    remote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Lock {
    bundles: Vec<Bundle>,
}

#[derive(Debug, Clone)]
struct Env {
    storage: PathBuf,
}

impl Default for Lock {
    fn default() -> Self {
        Lock {
            bundles: vec![],
        }
    }
}

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
        (@subcommand unlink =>
            (about: "unlink a bundle")
            (@arg BUNDLE: +required "bundle name")
        )
    )
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .get_matches();

    // default storage path
    let storage = dirs::home_dir()
        .ok_or(Error::HomedirNotFound)?
        .join(STORAGE_DIR);

    let env = Env { storage };

    match matches.subcommand() {
        ("add", Some(matches)) => {
            let bundle = matches.value_of("BUNDLE")
                .expect("Invalid: BUNDLE is required");

            let paths = matches.values_of("INPUT")
                .expect("Invalid: INPUT is required")
                .map(Path::new)
                .map(|it| it.canonicalize().expect("invalid path"))
                .collect::<Vec<PathBuf>>();

			cmd_add(&env, &bundle, &paths)?;
        },
        ("link", Some(matches)) => {
            let bundle = matches.value_of("BUNDLE")
                .expect("Invalid: BUNDLE is required");

            cmd_link(&env, &bundle)?;
        },
        _ => {},
    };

	Ok(())
}

fn get_storage_dir(env: &Env) -> Result<PathBuf> {
    let path = env.storage.clone();
    fs::create_dir_all(&path)?;

    Ok(path)
}

fn get_lockfile(env: &Env) -> Result<Lock> {
    let path = env.storage.join(LOCK_FILE);

    if !path.is_file() {
        return Ok(Default::default());
    }

    let contents = fs::read_to_string(path)?;
    let parsed = toml::from_str::<Lock>(&contents)?;

    Ok(parsed)
}

fn write_lockfile(env: &Env, lockfile: &Lock) -> Result<()> {
    let mut lock_path = get_storage_dir(&env)?;
    lock_path.push(LOCK_FILE);

    let lockfile_ser = toml::to_string(&lockfile)?;
    let mut out = File::create(lock_path)?;
    out.write_all(lockfile_ser.as_bytes())?;

    Ok(())
}

fn get_name(path: &PathBuf) -> Result<String> {
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

fn cmd_add(env: &Env, bundle_name: &str, paths: &Vec<PathBuf>) -> Result<()> {
    let mut lockfile = get_lockfile(&env)?;

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

    let bundle_path = env.storage
        .join("bundle")
        .join(bundle_name);

    if bundle_path.is_dir() {
        println!("adding to existing bundle `{}`", bundle_name);
    } else {
        println!("creating bundle `{}`", bundle_name);
        fs::create_dir_all(&bundle_path)?;
    }

    let entries = paths
        .iter()
        .map(|remote| {
            let remote_name = get_name(&remote)?;
            let local = bundle_path.join(remote_name);

            if remote.is_dir() {
                let mut options = fs_extra::dir::CopyOptions::new();
                options.copy_inside = true;
                options.overwrite = true;

                fs_extra::dir::copy(&remote, &local, &options)?;
            } else if remote.is_file() {
                fs::copy(&remote, &local)?;
            } else {
                println!("Does not exist: {:?}", remote);
            }

            let local = local
                .to_str()
                .expect("Invalid: paths should be UTF-8")
                .to_owned();

            let remote = remote
                .to_str()
                .expect("Invalid: paths should be UTF-8")
                .to_owned();

            Ok(Entry { local, remote })
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
        entries,
    };

    // Save the dotfile for the bundle itself, this has all the paths
    let dot_meta_path = bundle_path.join(BUNDLE_FILE);

    let bundle_ser = toml::to_string(&bundle)?;
    let mut out = File::create(dot_meta_path)?;
    out.write_all(bundle_ser.as_bytes())?;

    // Save the new dotfile, which contains only the paths that have
    // been linked successfully (which in this case should always be
    // all of them, but still)
    let linked = link(&bundle, &[], true)?;
    lockfile.bundles.push(Bundle { entries: linked, ..bundle });
    write_lockfile(&env, &lockfile)?;

    Ok(())
}

fn cmd_link(env: &Env, bundle_name: &str) -> Result<()> {
    let mut lockfile = get_lockfile(&env)?;

    let maybe_previous = lockfile.bundles.iter().find(|it| it.id == bundle_name);
    let previous_entries = if let Some(previous) = maybe_previous {
        previous.entries
            .iter()
            .map(|it| it.remote.as_ref())
            .collect::<Vec<_>>()
    } else {
        vec![]
    };

    let dir = env.storage.join(BUNDLE_DIR).join(bundle_name);
    if !dir.exists() || !dir.is_dir() {
        return Err(Error::BundleNotFound);
    }

    let dot_meta_path = dir.join(BUNDLE_FILE);
    if !dot_meta_path.exists() || !dot_meta_path.is_file() {
        return Err(Error::BundleMissingMeta);
    }

    let contents = fs::read_to_string(dot_meta_path)?;
    let bundle = toml::from_str::<Bundle>(&contents)?;

    let linked = link(&bundle, &previous_entries[..], false)?;
    lockfile.bundles.retain(|it| it.id != bundle_name);
    lockfile.bundles.push(Bundle { entries: linked, ..bundle });
    write_lockfile(&env, &lockfile)?;

    Ok(())
}

fn link(
    bundle: &Bundle,
    overwrite: &[&str],
    overwrite_all: bool
) -> Result<Vec<Entry>> {
    // TODO(happens): Check if linked bundles conflict with this one
    use std::os::unix::fs::symlink;

    let mut result = Vec::new();
    let mut overwrite_all = overwrite_all;
    for it in &bundle.entries {
        // first, make sure that all dirs leading up to the file
        // or dir exist (if there is no parent that means we
        // are placing the file at '/', which is fine, i guess?)
        let remote_path: PathBuf = it.remote.clone().into();
        let local_path: PathBuf = it.local.clone().into();

        if let Some(parent) = remote_path.parent() {
            // this is weird, but can happen
            if parent.is_file() {
                let text = format!(
                    "You're trying to link the file {}, but {} is a file. {}",
                    it.remote, parent.display(),
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

        if remote_path.exists() {
            if !overwrite_all && !overwrite.contains(&it.remote.as_ref()) {
                let choices = &["skip", "overwrite", "overwrite all"];
                let selection = Select::with_theme(&theme::ColorfulTheme::default())
                    .with_prompt(&format!("{} already exists.", it.remote))
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
            if remote_path.is_dir() {
                fs::remove_dir_all(&remote_path)?;
            } else if remote_path.is_file() {
                fs::remove_file(&remote_path)?;
            }
        }

        symlink(&local_path, &remote_path)?;
        result.push(it.clone());
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    const TEST_ROOT: &'static str = "test_tmp";
    use super::*;
    use std::fs::File;

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

    #[test]
    fn cmd_add_should_work_for_new_bundle() -> Result<()> {
        let (env, config_dir) = setup();

        let paths = vec![
            config_dir.join("a"),
            config_dir.join("b"),
        ];

        cmd_add(&env, "test_bundle", &paths)?;

        // TODO(happens): Add more assertions

        clean();
        Ok(())
    }

    // Returns a temp env and a folder with test files
    // ./test_tmp
    //     /dotgirl (storage dir)
    //     /config
    //         /a
    //             /sub
    //                 config
    //             config
    //         /b
    //             config
    //
    fn setup() -> (Env, PathBuf) {
        let mut tmp = std::env::current_dir().expect("No current dir");
        tmp.push(TEST_ROOT);

        let storage = tmp.join(STORAGE_DIR);
        fs::create_dir_all(&storage).expect("Could not create storage dir");

        let conf = tmp.join("config");

        let conf_a = conf.join("a");
        let conf_a_sub = conf_a.join("sub");
        fs::create_dir_all(&conf_a_sub).expect("Could not create test dir");

        let mut conf_a_file = File::create(conf_a.join("config"))
            .expect("Could not create test file");
        conf_a_file.write_all(b"hello config")
            .expect("Could not write to test file");
        let mut conf_a_sub_file = File::create(conf_a_sub.join("config"))
            .expect("Could not create test file");
        conf_a_sub_file.write_all(b"hello config")
            .expect("Could not write to test file");

        let conf_b = conf.join("b");
        fs::create_dir_all(&conf_b).expect("Could not create test dir");
        let mut conf_b_file = File::create(conf_b.join("config"))
            .expect("Could not create test file");
        conf_b_file.write_all(b"hello config")
            .expect("Could not write to test file");

        (Env { storage }, conf)
    }

    fn clean() {
        let mut tmp = std::env::current_dir().expect("No current dir");
        tmp.push(TEST_ROOT);
        fs::remove_dir_all(&tmp).expect("Couldn't clean up test root");
    }
}

