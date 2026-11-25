use std::{
    io::{stdin, stdout, Write},
    option::Option,
    path::{Path, PathBuf},
};

use anyhow_ext::{Context, Result};
use smartstring::alias::String;
use swmm_core::{core, mods::LookupMod};
use uk_mod::{unpack::ModReader, Manifest, Meta, ModOption, OptionGroup};

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
        cmd status {}
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
    Status(Status),
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

#[derive(Debug)]
pub struct Status;

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
        stdin().read_line(&mut _input).unwrap_or_default();
        _input
    }};
}

#[derive(Debug)]
pub struct Runner {
    core: core::Manager,
    cli:  Swmm,
}

impl Runner {
    pub fn new(cli: Swmm) -> Result<Self> {
        Ok(Self {
            core: core::Manager::init().context("SWMM couldn't load its settings and profiles")?,
            cli,
        })
    }

    fn check_mod(&self, path: &Path) -> Result<Option<PathBuf>> {
        let (mod_, converted) = jobs::read_for_install(&self.core, path)?;
        if converted.wiiu {
            println!("{}: converted from Wii U. Some parts may not work.", mod_.meta.name);
        }
        if converted.rebased {
            println!("{} was made for another version of the game. Its lists of actors, flags and texts were moved to this version.", mod_.meta.name);
        }
        if converted.code_patches {
            println!(
                "{} also has code patches (ExeFS, IPS or pchtxt). Savage Wind can't run them, so only its game files are installed and anything that needs those patches won't work.",
                mod_.meta.name
            );
        }
        println!("Installing {}...", mod_.meta.name);
        let _ = ModReader::open_peek(&mod_.path, vec![]).context("Failed to open the converted mod")?;
        Ok(Some(mod_.path))
    }

    fn status(&self) -> Result<()> {
        let settings = self.core.settings();
        let game = swmm_core::savage_wind::current_game_files(&settings).ok();
        let mods = self
            .core
            .mod_manager()
            .all_mods()
            .enumerate()
            .map(|(index, m)| {
                serde_json::json!({
                    "index": index,
                    "name": m.meta.name.as_str(),
                    "version": m.meta.version.as_str(),
                    "author": m.meta.author.as_str(),
                    "enabled": m.enabled,
                    "came_as": swmm_core::origin::describe(&m),
                    "options": m.enabled_options.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();
        let status = serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "profile": self.core.mod_manager().current_profile(),
            "executable": settings.savage_wind.executable,
            "game_version": game.as_ref().and_then(|g| g.version.clone()),
            "dlc_version": game.as_ref().and_then(|g| g.aoc_version.clone()),
            "overlay": swmm_core::savage_wind::overlay_dir(&settings),
            "pending_deploy": self.core.deploy_manager().pending(),
            "mods": mods,
        });
        println!("{}", serde_json::to_string_pretty(&status)?);
        Ok(())
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
                    let options = default_options(&mod_.meta);
                    let manifest = if options.is_empty() {
                        mod_.manifest()?
                    } else if profile.is_none() {
                        let names: Vec<&str> = options.iter().map(|o| o.name.as_str()).collect();
                        println!("Options picked: {}", names.join(", "));
                        mods.set_enabled_options(mod_.as_map_id(), options)?
                    } else {
                        println!("The mod's options can only be picked for the current profile; none were picked");
                        mod_.manifest()?
                    };
                    mods.save()?;
                    println!("Applying mod to load order...");
                    let deployer = self.core.deploy_manager();
                    deployer.apply(Some(manifest.as_ref().clone()))?;
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
                    println!("Removing mod {}...", mod_.meta.name);
                    mod_manager.del(mod_, profile.as_ref())?;
                    mod_manager.save()?;
                    manifests.extend(mod_.manifest()?.as_ref());
                } else {
                    println!("Installed mods:");
                    for (i, mod_) in mods.iter().enumerate() {
                        println!(
                            "{}. {} (v{}) by {}",
                            i + 1,
                            mod_.meta.name,
                            mod_.meta.version,
                            mod_.meta.author
                        );
                    }
                    print!("Enter mod(s) to uninstall, separated by commas: ");
                    let mut manifests = Manifest::default();
                    for id in input!().replace(' ', "").split(',') {
                        let mod_ = mods
                            .get(id.trim().parse::<usize>().context("Invalid mod number")? - 1)
                            .with_context(|| format!("Mod {} does not exist", id))?;
                        println!("Removing mod {}...", mod_.meta.name);
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
            SwmmCmd::Status(_) => self.status()?,
        };
        Ok(())
    }
}

fn default_options(meta: &Meta) -> Vec<ModOption> {
    let mut picked = Vec::new();
    for group in &meta.options {
        match group {
            OptionGroup::Exclusive(group) => {
                let chosen = group
                    .default
                    .as_ref()
                    .and_then(|path| group.options.iter().find(|o| &o.path == path))
                    .or_else(|| group.required.then(|| group.options.first()).flatten());
                picked.extend(chosen.cloned());
            }
            OptionGroup::Multiple(group) => {
                let before = picked.len();
                picked.extend(group.options.iter().filter(|o| group.defaults.contains(&o.path)).cloned());
                if picked.len() == before && group.required {
                    picked.extend(group.options.first().cloned());
                }
            }
        }
    }
    picked
}
