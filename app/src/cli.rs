use std::{
    io::{stdin, stdout, Write},
    option::Option,
    path::{Path, PathBuf},
};

use anyhow_ext::{Context, Result};
use smartstring::alias::String;
use swmm_core::{core, mods::LookupMod};
use swmm_modpack::{unpack::ModReader, Manifest, Meta};

use crate::ui::jobs;

xflags::xflags! {
    src "./src/cli.rs"

    cmd swmm {
        optional -d, --debug
        optional -p, --portable
        optional -D, --deploy
        cmd install {
            required path: PathBuf
            optional profile: String
        }
        cmd package {
            required path: PathBuf
            required output: PathBuf
            required meta: PathBuf
        }
        cmd uninstall {
            optional index: usize
            optional profile: String
        }
        cmd remerge {}
        cmd deploy {}
    }
}
#[derive(Debug)]
pub struct Swmm {
    pub debug: bool,
    #[allow(dead_code)]
    pub portable: bool,
    pub deploy: bool,
    pub subcommand: SwmmCmd,
}

#[derive(Debug)]
pub enum SwmmCmd {
    Install(Install),
    Uninstall(Uninstall),
    Package(Package),
    Remerge(Remerge),
    Deploy(Deploy),
}

#[derive(Debug)]
pub struct Install {
    pub path:    PathBuf,
    pub profile: Option<String>,
}

#[derive(Debug)]
pub struct Package {
    pub path:   PathBuf,
    pub output: PathBuf,
    pub meta:   PathBuf,
}

#[derive(Debug)]
pub struct Uninstall {
    pub index:   Option<usize>,
    pub profile: Option<String>,
}

#[derive(Debug)]
pub struct Remerge;

#[derive(Debug)]
pub struct Deploy;

impl Swmm {
    #[allow(dead_code)]
    pub fn from_env_or_exit() -> Self {
        Self::from_env_or_exit_()
    }

    #[allow(dead_code)]
    pub fn from_env() -> xflags::Result<Self> {
        Self::from_env_()
    }

    #[allow(dead_code)]
    pub fn from_vec(args: Vec<std::ffi::OsString>) -> xflags::Result<Self> {
        Self::from_vec_(args)
    }
}

macro_rules! input {
    () => {{
        stdout().flush()?;
        let mut _input = std::string::String::new();
        stdin().read_line(&mut _input).unwrap();
        _input
    }};
}

#[derive(Debug)]
pub struct Runner {
    core: core::Manager,
    cli:  Swmm,
}

impl Runner {
    pub fn new(cli: Swmm) -> Self {
        Self {
            core: core::Manager::init().unwrap(),
            cli,
        }
    }

    fn check_mod(&self, path: &Path) -> Result<Option<PathBuf>> {
        let (mod_, path) = match ModReader::open(path, vec![]) {
            Ok(mod_) => (mod_, path.to_path_buf()),
            Err(e) => {
                match swmm_core::mods::convert_gfx(&self.core, path, None) {
                    Ok(path) => {
                        log::info!("Opening mod at {}", path.display());
                        (
                            ModReader::open(&path, vec![])
                                .context("Failed to open converted mod")?,
                            path,
                        )
                    }
                    Err(e2) => {
                        anyhow_ext::bail!(
                            "Could not open mod. Error when attempting to open as a mod package: {}. \
                             Error when attempting to open as legacy mod: {}.",
                            e,
                            e2
                        )
                    }
                }
            }
        };
        if !mod_.meta.options.is_empty() {
            anyhow_ext::bail!(
                "This mod contains configuration options and should be installed via the GUI."
            );
        }
        println!("Installing {}...", mod_.meta.name);
        Ok(Some(path))
    }

    fn deploy(&self) -> Result<()> {
        let deployer = self.core.deploy_manager();
        if deployer.pending() {
            println!("Deploying changes...");
            deployer.deploy()?;
            println!("Deployment complete");
        } else {
            println!("No changes pending deployment");
        };
        Ok(())
    }

    pub fn run(self) -> Result<()> {
        if self.cli.debug {
            env_logger::init();
            log::set_max_level(log::LevelFilter::Debug);
        }
        match &self.cli.subcommand {
            SwmmCmd::Install(Install { path, profile }) => {
                if let Some(path) = self.check_mod(path)? {
                    let mods = self.core.mod_manager();
                    let mod_ = mods.add(&path, profile.as_ref())?;
                    mods.set_enabled(mod_.as_map_id(), true, profile.as_ref())?;
                    mods.save()?;
                    println!("Applying mod to load order...");
                    let deployer = self.core.deploy_manager();
                    deployer.apply(Some(mod_.manifest()?.as_ref().clone()))?;
                    if self.cli.deploy {
                        self.deploy()?;
                    }
                    println!("Done!");
                }
            }
            SwmmCmd::Package(pkg) => {
                println!("Packaging mod...");
                let builder = jobs::PackageJob {
                    source: pkg.path.clone(),
                    dest:   pkg.output.clone(),
                    meta:   Meta::parse(&pkg.meta)?,
                };
                jobs::package_mod(&self.core, builder)?;
                println!("Done!");
            }
            SwmmCmd::Remerge(_) => {
                println!("Remerging...");
                jobs::apply_changes(&self.core, vec![], None)?;
                println!("Done!");
            }
            SwmmCmd::Uninstall(Uninstall { index, profile }) => {
                let mut manifests = Manifest::default();
                let mod_manager = self.core.mod_manager();
                let mods = mod_manager.mods().collect::<Vec<_>>();

                if let Some(index_value) = index {
                    let mod_ = mods
                        .get(*index_value)
                        .with_context(|| format!("Mod {} does not exist", index_value))?;
                    println!("Removing mod {}...", &mod_.meta.name);
                    mod_manager.del(mod_, profile.as_ref())?;
                    mod_manager.save()?;
                    manifests.extend(mod_.manifest()?.as_ref());
                } else {
                    println!("Installed mods:");
                    for (i, mod_) in mods.iter().enumerate() {
                        println!(
                            "{}. {} (v{}) by {}",
                            i + 1,
                            &mod_.meta.name,
                            &mod_.meta.version,
                            &mod_.meta.author
                        );
                    }
                    print!("Enter mod(s) to uninstall, separated by commas: ");
                    let mut manifests = Manifest::default();
                    for id in input!().replace(' ', "").split(',') {
                        let mod_ = mods
                            .get(id.trim().parse::<usize>().context("Invalid mod number")? - 1)
                            .with_context(|| format!("Mod {} does not exist", id))?;
                        println!("Removing mod {}...", &mod_.meta.name);
                        mod_manager.del(mod_, profile.as_ref())?;
                        mod_manager.save()?;
                        manifests.extend(mod_.manifest()?.as_ref());
                    }
                }

                println!("Applying changes to merge...");
                self.core.deploy_manager().apply(Some(manifests))?;
                if self.cli.deploy {
                    self.deploy()?;
                }
                println!("Done!");
            }
            SwmmCmd::Deploy(_) => self.deploy()?,
        };
        Ok(())
    }
}
