use std::path::{Path, PathBuf};
use clap::{clap_app, AppSettings};
use serde::{Serialize, Deserialize};
use dialoguer::{Confirmation, Select, theme};

mod util;
mod disk;

use disk::{Disk, Filesystem as _};

const STORAGE_DIR: &'static str = "dotgirl";
const BUNDLE_DIR: &'static str = "bundle";

// const CONFIG_FILE: &'static str = "config.toml";
const LOCK_FILE: &'static str = "lock.toml";
const BUNDLE_FILE: &'static str = "bundle.toml";

#[derive(Debug)]
pub enum Error {
    IoError(std::io::Error),
    IoExtraError(fs_extra::error::Error),
    HomedirNotFound,
    ParseError(toml::de::Error),
    SerializeError(toml::ser::Error),
    LastComponentInvalid(String),
    BundleNotFound,
    BundleMissingMeta,
    Simple(&'static str),
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
    Disk::mkdir_all(&env.storage)?;

    Ok(path)
}

fn get_lockfile(env: &Env) -> Result<Lock> {
    let path = env.storage.join(LOCK_FILE);

    if !Disk::is_file(&path) {
        return Ok(Default::default());
    }

    let contents = Disk::get(path)?;
    let parsed = toml::from_str::<Lock>(&contents)?;

    Ok(parsed)
}

fn write_lockfile(env: &Env, lockfile: &Lock) -> Result<()> {
    let mut lock_path = get_storage_dir(&env)?;
    lock_path.push(LOCK_FILE);

    let ser = toml::to_string(&lockfile)?;
    Disk::put(&lock_path, &ser)?;

    Ok(())
}

fn cmd_add(env: &Env, bundle_name: &str, paths: &Vec<PathBuf>) -> Result<()> {
    let mut lockfile = get_lockfile(&env)?;

    // Filter out symlinks
    // TODO(happens): More validation
    //   - Check that directories don't contain each other
    //   - Check for duplicates
    //   - Exclude storage directory
    let paths = paths
        .into_iter()
        .filter(|it| !Disk::is_symlink(&it))
        .collect::<Vec<_>>();

    let bundle_path = env.storage
        .join("bundle")
        .join(bundle_name);

    if bundle_path.is_dir() {
        println!("adding to existing bundle `{}`", bundle_name);
    } else {
        println!("creating bundle `{}`", bundle_name);
        Disk::mkdir_all(&bundle_path)?;
    }

    let entries = paths
        .iter()
        .map(|remote| {
            let remote_name = util::get_name(&remote)?;
            let local = bundle_path.join(remote_name);

            Disk::copy(&remote, &local)?;
            Disk::remove(&remote)?;

            let local = format!("{}", local.display());
            let remote = format!("{}", remote.display());

            Ok(Entry { local, remote })
        })
        .filter_map(Result::ok)
        .collect::<Vec<Entry>>();

    // TODO(happens): Report on skipped

    let bundle = Bundle {
        id: String::from(bundle_name),
        entries,
    };

    // Save the dotfile for the bundle itself, this has all the paths
    let dot_meta_path = bundle_path.join(BUNDLE_FILE);

    let ser = toml::to_string(&bundle)?;
    Disk::put(&dot_meta_path, &ser)?;

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
    if !Disk::is_dir(&dir) {
        return Err(Error::BundleNotFound);
    }

    let dot_meta_path = dir.join(BUNDLE_FILE);
    if !Disk::is_file(&dot_meta_path) {
        return Err(Error::BundleMissingMeta);
    }

    let contents = Disk::get(&dot_meta_path)?;
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
            if Disk::is_file(&parent) {
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
                    Disk::remove(&parent)?;
                }
            }

            if !parent.is_dir() {
                Disk::mkdir_all(&parent)?;
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
            Disk::remove(&remote_path)?;
        }

        Disk::symlink(&local_path, &remote_path)?;
        result.push(it.clone());
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_add_should_work_for_new_bundle() {
        let (env, config_dir) = setup();

        let paths = vec![
            config_dir.join("a"),
            config_dir.join("b"),
        ];

        cmd_add(&env, "test_bundle", &paths).expect("Add should have worked");

        Disk::print();

        assert!(Disk::is_symlink(config_dir.join("a")));
        assert!(Disk::is_symlink(config_dir.join("b")));

        let bundle_dir = env.storage.join("bundle/test_bundle");

        assert!(Disk::is_dir(bundle_dir.join("a")));
        assert!(Disk::is_dir(bundle_dir.join("a/sub")));
        assert!(Disk::is_dir(bundle_dir.join("b")));

        assert!(Disk::is_file(bundle_dir.join("a/config")));
        assert!(Disk::is_file(bundle_dir.join("a/sub/config")));
        assert!(Disk::is_file(bundle_dir.join("a/.hidden-config")));

        clean();
    }

    #[test]
    fn cmd_add_should_trim_dot_prefix() {
        let (env, config_dir) = setup();
        let paths = vec![config_dir.join(".hidden-config")];

        cmd_add(&env, "test_bundle", &paths).expect("Add should have worked");

        Disk::print();

        assert!(Disk::is_symlink(config_dir.join(".hidden-config")));
        assert!(Disk::is_file(env.storage.join("bundle/test_bundle/hidden-config")));

        clean();
    }

    // Returns a temp env and a folder with test files
    // ./test_tmp
    //     /dotgirl (storage dir)
    //     /config
    //         config
    //         .hidden-config
    //         /a
    //             /sub
    //                 config
    //                 .hidden-config
    //             config
    //         /b
    //             config
    //
    fn setup() -> (Env, PathBuf) {
        let root = PathBuf::from("/");

        let storage = root.join(STORAGE_DIR);
        Disk::mkdir_all(&storage).unwrap();

        let conf = root.join("config");

        let conf_a = conf.join("a");
        let conf_a_sub = conf_a.join("sub");
        Disk::mkdir_all(&conf_a_sub).unwrap();

        Disk::put(conf.join("config"), "hello config").unwrap();
        Disk::put(conf.join(".hidden-config"), "hello config").unwrap();

        Disk::put(conf_a.join("config"), "hello config").unwrap();
        Disk::put(conf_a.join(".hidden-config"), "hello config").unwrap();
        Disk::put(conf_a_sub.join("config"), "hello config").unwrap();

        let conf_b = conf.join("b");
        Disk::mkdir_all(&conf_b).unwrap();
        Disk::put(conf_b.join("config"), "hello config").unwrap();

        (Env { storage }, conf)
    }

    fn clean() {
        Disk::clear();
    }
}

