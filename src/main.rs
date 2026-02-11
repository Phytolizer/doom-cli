use std::fs::create_dir_all;
use std::path::Path;
use std::path::PathBuf;
use std::process::exit;
use std::process::Command;
use std::str::FromStr;
use std::sync::Mutex;

use clap::Parser;
use dialoguer::console::style;
use dialoguer::theme::ColorfulTheme;
use dialoguer::Confirm;
use dialoguer::Input;
use dialoguer::MultiSelect;
use itertools::Itertools;
use log::error;
use log::info;
use log::warn;
use once_cell::sync::Lazy;

use crate::cmd::CommandLine;
use crate::cmd::Line;
use crate::engine::read_known_engines;
use crate::engine::DoomEngineKind;
use crate::error::Error;
use crate::pwads::parse_arg_pwads;
use crate::pwads::Pwads;
use crate::render::batch_render;
use crate::util::absolute_path;

mod autoload;
mod cmd;
mod engine;
mod error;
mod job;
mod pwads;
mod render;
mod score;
mod search;
mod util;

static CUSTOM_DOOM_DIR: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

enum FileType {
    Iwad,
    Pwad,
    Demo,
}

impl FileType {
    fn get_search_dirs(&self) -> Result<Vec<PathBuf>, Error> {
        vec![doom_dir(), Ok(public_doom_dir())]
            .into_iter()
            .collect()
    }
}

fn home_dir() -> Result<PathBuf, Error> {
    dirs::home_dir().ok_or(Error::Homeless)
}

fn doom_dir() -> Result<PathBuf, Error> {
    if let Some(dir) = CUSTOM_DOOM_DIR.lock().unwrap().as_ref() {
        Ok(dir.clone())
    } else {
        home_dir().map(|h| h.join("doom"))
    }
}

fn public_doom_dir() -> PathBuf {
    PathBuf::from("/public/doom")
}

fn demo_dir() -> Result<PathBuf, Error> {
    doom_dir().map(|d| d.join("demo"))
}

fn dump_dir() -> Result<PathBuf, Error> {
    doom_dir().map(|d| d.join("demo").join("render"))
}

fn select_between<P: AsRef<Path>>(
    search: impl AsRef<Path>,
    options: impl AsRef<[P]>,
) -> Result<Vec<PathBuf>, Error> {
    MultiSelect::new()
        .with_prompt(format!("Multiple files were found for the search term {}. Please select one or more of the following:", search.as_ref().display()))
        .items(
            &options
                .as_ref()
                .iter()
                .map(|opt| opt.as_ref().to_string_lossy())
                .collect::<Vec<_>>(),
        )
        .interact()
        .map(|indices| indices.iter().map(|i| options.as_ref()[*i].as_ref().to_owned()).collect())
        .map_err(Error::Dialoguer)
}

fn run_doom<'l>(mut cmdline: impl Iterator<Item = &'l str>) -> Result<(), Error> {
    let binary = PathBuf::from(cmdline.next().unwrap());
    if !binary.exists() {
        return Err(Error::FileNotFound(binary.to_string_lossy().into_owned()));
    }
    let binary_dir = dirname(&binary);
    let args = cmdline
        .filter_map(|arg| {
            let trimmed = arg.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect::<Vec<_>>();
    Command::new(binary)
        .args(args)
        .current_dir(binary_dir)
        .status()
        .map(|_| ())
        .map_err(Error::RunningDoom)
}

fn dirname(binary: &Path) -> PathBuf {
    let mut d = binary.to_owned();
    d.pop();
    d
}

#[derive(Debug, clap::Parser)]
#[clap(version(clap::crate_version!()))]
/// Command-line Doom launcher
struct Args {
    #[clap(short = 'c', long, value_name = "COMPLEVEL")]
    compatibility_level: Option<String>,
    #[clap(short = 'G', long)]
    debug: bool,
    #[clap(long, value_name = "DIR")]
    doom_dir: Option<PathBuf>,
    #[clap(short = 'e', long)]
    engine: Option<String>,
    #[clap(short = 'i', long, value_name = "PATH")]
    iwad: Option<PathBuf>,
    #[clap(short = 'n', long)]
    no_confirm: bool,
    #[clap(long)]
    pistol_start: bool,
    #[clap(short = 'd', long, value_name = "DEMO")]
    play_demo: Option<PathBuf>,
    #[clap(short = 'p', long, value_name = "PATH")]
    pwads: Vec<PathBuf>,
    #[clap(short = 'r', long, value_name = "DEMO")]
    record: Option<PathBuf>,
    #[clap(short = 'R', long, value_name = "DEMO")]
    render: Option<String>,
    #[clap(long)]
    short_tics: bool,
    #[clap(short = 'w', long, value_name = "LEVEL")]
    warp: Option<String>,
    #[clap(last = true, value_name = "ARGS")]
    /// Arguments that will be passed directly to the chosen engine,
    /// without any processing.
    rest: Vec<String>,
}

fn run() -> Result<(), Error> {
    let args = Args::parse();

    if let Some(doom_dir) = args.doom_dir {
        *CUSTOM_DOOM_DIR.lock().unwrap() = Some(doom_dir);
    }

    if !doom_dir()?.exists() {
        let answer = Confirm::with_theme(&ColorfulTheme::default())
            .with_prompt(format!(
                "You don't have a dedicated Doom directory at {}. Create it?",
                doom_dir()?.to_string_lossy()
            ))
            .interact()
            .map_err(Error::Dialoguer)?;
        if answer {
            create_dir_all(doom_dir()?).map_err(Error::Io)?;
            info!("Success.");
        } else {
            warn!("Cannot continue. You can set the dedicated Doom directory by passing the flag --doom-dir. You only have to pass the flag once, as it will be remembered.");
            return Ok(());
        }
    }

    let known_engines = read_known_engines()?;
    let engine_name = args
        .engine
        .or_else(|| known_engines.iter().next())
        .ok_or(Error::NoEngines)?;
    let engine = &known_engines.get(&engine_name).unwrap_or_else(|| {
        error!("ERROR: Unknown sourceport '{}'", engine_name);
        exit(-1);
    });

    let mut search_iwads: Box<dyn Iterator<Item = PathBuf>> = args
        .iwad
        .map::<Box<dyn Iterator<Item = PathBuf>>, _>(|i| Box::new(std::iter::once(i)))
        .unwrap_or_else(|| {
            Box::new(
                ["DOOM2.WAD", "DOOM.WAD", "DOOMU.WAD", "DOOM1.WAD"]
                    .into_iter()
                    .map(|i: &str| PathBuf::from_str(i).unwrap()),
            )
        });
    let iwad_path = loop {
        let iwad = match search_iwads.next() {
            Some(i) => i,
            None => break None,
        };
        let iwad_path = search::search_file(&iwad, FileType::Iwad).or_else(|e| {
            if let Error::FileNotFound(_) = e {
                Ok(vec![])
            } else {
                Err(e)
            }
        })?;
        if iwad_path.is_empty() {
            warn!("IWAD not found: '{}'", iwad.display());
        } else {
            break Some(iwad_path);
        }
    };
    if iwad_path.is_none() {
        error!("No IWADs could be found.");
        exit(-1);
    }
    let iwad_path = iwad_path.unwrap();
    let iwad_path = absolute_path(&iwad_path[0])?;
    let iwad = iwad_path.to_string_lossy().to_string();

    let iwad_base = iwad_path
        .file_name()
        .ok_or_else(|| Error::NoFileStem(iwad_path.to_string_lossy().into_owned()))
        .and_then(|f| {
            f.to_str()
                .ok_or_else(|| Error::NonUtf8Path(f.to_string_lossy().into_owned()))
        })?;
    let iwad_noext = iwad_path
        .file_stem()
        .ok_or_else(|| Error::NoFileStem(iwad_path.to_string_lossy().into_owned()))
        .and_then(|i| {
            i.to_str()
                .ok_or_else(|| Error::NonUtf8Path(i.to_string_lossy().into_owned()))
        })?
        .to_lowercase();

    let mut cmdline = CommandLine::new();
    if args.debug {
        cmdline.push_line(Line::from_word("/usr/bin/lldb", 0));
    }
    cmdline.push_line(Line::from_word(
        engine
            .binary
            .to_str()
            .ok_or_else(|| Error::NonUtf8Path(engine.binary.to_string_lossy().into_owned()))?,
        0,
    ));
    if args.debug {
        cmdline.push_line(Line::from_word("--", 0));
    }
    if !engine.required_args.is_empty() {
        cmdline.push_line(Line::from_words(&engine.required_args, 1));
    }
    cmdline.push_line(Line::from_words(&["-iwad", &iwad], 1));

    let mut pwads = Pwads::new();

    autoload::autoload(&mut pwads, &engine.binary, &iwad_noext)?;

    let mut viddump_folder_name = vec![];

    parse_arg_pwads(&args.pwads, &mut viddump_folder_name, &mut pwads)?;

    if !pwads.wads().is_empty() {
        cmdline.push_line(Line::from_word(
            if engine.use_merge_arg {
                "-merge"
            } else {
                "-file"
            },
            1,
        ));
        pwads.wads().iter().try_for_each(|pwad| {
            pwad.to_str()
                .ok_or_else(|| Error::NonUtf8Path(pwad.to_string_lossy().into_owned()))
                .map(|pwad| cmdline.push_line(Line::from_word(pwad, 2)))
        })?;
    }

    if !pwads.dehs().is_empty() {
        if engine.use_merge_arg {
            if pwads.wads().is_empty() {
                cmdline.push_line(Line::from_word("-merge", 1));
            }
        } else {
            cmdline.push_line(Line::from_word("-deh", 1));
        }
        pwads.dehs().iter().try_for_each(|deh| {
            deh.to_str()
                .ok_or_else(|| Error::NonUtf8Path(deh.to_string_lossy().into_owned()))
                .map(|deh| cmdline.push_line(Line::from_word(deh, 2)))
        })?;
    }

    if let Some(complevel) = args.compatibility_level.as_ref() {
        cmdline.push_line(Line::from_words(
            &[String::from("-complevel"), complevel.to_string()],
            1,
        ));
    }

    if args.pistol_start {
        cmdline.push_line(Line::from_word("-pistolstart", 1));
    }

    let skill_param = if engine.kind == DoomEngineKind::ZDoom {
        &["+skill", "3"]
    } else {
        &["-skill", "4"]
    };

    if let Some(recording_demo) = args.record.as_ref() {
        let demo_path = PathBuf::from(recording_demo);
        let demo_path = if demo_path.is_absolute() {
            demo_path
        } else {
            demo_dir()?.join(demo_path)
        };
        cmdline.push_line(Line::from_word("-record", 1));
        cmdline.push_line(Line::from_word(demo_path.to_string_lossy(), 2));
        if !args.short_tics {
            cmdline.push_line(Line::from_word("-longtics", 1));
        }
    } else if args.short_tics {
        cmdline.push_line(Line::from_word("-shorttics", 1));
    }

    if let Some(playing_demo) = args.play_demo.as_ref() {
        let demo = select_between(
            playing_demo,
            search::search_file(playing_demo, FileType::Demo)?,
        )?;
        if demo.is_empty() {
            error!("No such demo: {}", playing_demo.display());
            exit(-1);
        }
        cmdline.push_line(Line::from_word("-playdemo", 1));
        cmdline.push_line(Line::from_word(
            demo[0]
                .to_str()
                .ok_or_else(|| Error::NonUtf8Path(demo[0].to_string_lossy().into_owned()))?,
            2,
        ));
    }

    if let Some(warp) = args.warp.as_ref() {
        cmdline.push_line(Line::from_words(
            &{
                let mut words = vec!["-warp"];
                words.extend(warp.split_ascii_whitespace());
                words
            },
            1,
        ));
    }

    if args.warp.is_some() {
        cmdline.push_line(Line::from_words(skill_param, 1));
    }

    if !args.rest.is_empty() {
        cmdline.push_line(Line::from_words(&args.rest, 1));
    }

    if let Some(render_matches) = args.render.as_ref() {
        let dump_dir = dump_dir()?
            .join(iwad_base)
            .join(viddump_folder_name.join(","));
        let renderings = render::collect_renderings(render_matches, &dump_dir)?;
        batch_render(renderings, &cmdline, dump_dir)?;
    } else {
        eprintln!();
        eprintln!(
            "Command line: \n'\n{}\n'",
            cmdline.iter_lines().map(|l| l.iter().join(" ")).join("\n")
        );
        if !args.no_confirm {
            Input::<String>::with_theme(&ColorfulTheme {
                prompt_prefix: style("*".into()).yellow(),
                ..Default::default()
            })
            .with_prompt("Press enter to launch Doom.")
            .allow_empty(true)
            .interact()?;
        }
        run_doom(cmdline.iter_words())?;
    }
    Ok(())
}

fn main() {
    pretty_env_logger::init();
    if let Err(e) = run() {
        error!("{}", e);
        exit(-1);
    }
}
