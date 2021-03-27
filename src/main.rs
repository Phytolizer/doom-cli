use std::collections::HashMap;
use std::fmt::Display;
use std::fs::create_dir_all;
use std::fs::File;
use std::io;
use std::io::stdin;
use std::io::stdout;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::exit;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::mpsc::channel;
use std::thread::sleep;
use std::time::Duration;

use clap::App;
use clap::AppSettings;
use clap::Arg;
use dialoguer::MultiSelect;
use itertools::Itertools;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde::Serialize;
use walkdir::WalkDir;

use crate::cmd::CommandLine;
use crate::cmd::Line;
use crate::engine::read_known_engines;
use crate::engine::DoomEngineKind;
use crate::job::Job;
use crate::util::absolute_path;

mod cmd;
mod engine;
mod job;
mod util;

struct Pwads {
    wads: Vec<PathBuf>,
    dehs: Vec<PathBuf>,
}

impl Pwads {
    fn new() -> Self {
        Self {
            wads: vec![],
            dehs: vec![],
        }
    }

    fn add_wads(&mut self, mut wads: Vec<PathBuf>) {
        self.wads.append(&mut wads);
    }

    fn add_wad(&mut self, wad: impl AsRef<Path>) {
        self.wads.push(wad.as_ref().to_owned());
    }

    fn add_dehs(&mut self, mut dehs: Vec<PathBuf>) {
        self.dehs.append(&mut dehs);
    }

    fn add_deh(&mut self, deh: PathBuf) {
        self.dehs.push(deh);
    }

    fn wads(&self) -> &[PathBuf] {
        &self.wads
    }

    fn dehs(&self) -> &[PathBuf] {
        &self.dehs
    }
}

enum FileType {
    Iwad,
    Pwad,
    Demo,
}

impl FileType {
    fn get_search_dirs(&self) -> Result<Vec<PathBuf>, Error> {
        match self {
            FileType::Iwad => vec![doom_dir()].into_iter().collect(),
            FileType::Pwad => vec![doom_dir()].into_iter().collect(),
            FileType::Demo => vec![doom_dir()].into_iter().collect(),
        }
    }
}

#[cfg(unix)]
const ARG_SEPARATOR: char = ':';
#[cfg(windows)]
const ARG_SEPARATOR: char = ';';

fn home_dir() -> Result<PathBuf, Error> {
    dirs::home_dir().ok_or(Error::Homeless)
}

fn doom_dir() -> Result<PathBuf, Error> {
    home_dir().map(|h| h.join("doom"))
}

fn demo_dir() -> Result<PathBuf, Error> {
    doom_dir().map(|d| d.join("demo"))
}

#[cfg(unix)]
static DUMP_DIR: Lazy<PathBuf> = Lazy::new(|| {
    let raw_output = String::from_utf8(
        Command::new("findmnt")
            .arg("/dev/sdd1")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let second_line = raw_output.lines().nth(1).unwrap_or_else(|| {
        eprintln!("Please mount /dev/sdd1. I beg you.");
        exit(-1);
    });
    second_line.split_whitespace().next().unwrap().into()
});

#[cfg(windows)]
static DUMP_DIR: Lazy<PathBuf> = Lazy::new(|| PathBuf::from("E:").join("Videos"));

fn search_files(list: &[String], ty: FileType) -> Result<Vec<PathBuf>, Error> {
    list.iter()
        .map(|i| {
            search_file_in_dirs_by(PathBuf::from(i), ty.get_search_dirs()?, |p| {
                ["wad", "deh", "bex", "pk3", "pk7", "pke", "zip"].contains(
                    &p.extension()
                        .map(|ext| ext.to_string_lossy().to_string())
                        .unwrap_or_default()
                        .as_str(),
                )
            })
        })
        .map(|rr| rr.map(|r| r.into_iter().next().unwrap()))
        .collect()
}

fn search_file(name: impl AsRef<str>, ty: FileType) -> Result<Vec<PathBuf>, Error> {
    search_file_in_dirs_by(name.as_ref().into(), ty.get_search_dirs()?, |_| true)
}

fn search_file_by(
    name: impl AsRef<str>,
    ty: FileType,
    predicate: impl Fn(&Path) -> bool,
) -> Result<Vec<PathBuf>, Error> {
    search_file_in_dirs_by(name.as_ref().into(), ty.get_search_dirs()?, predicate)
}

fn search_file_in_dirs_by(
    name: PathBuf,
    search_dirs: Vec<PathBuf>,
    predicate: impl Fn(&Path) -> bool,
) -> Result<Vec<PathBuf>, Error> {
    if name.is_absolute() {
        let mut parent = name.clone();
        parent.pop();
        search_file_in_dirs_by(
            PathBuf::from(
                name.file_stem()
                    .ok_or_else(|| Error::NoFileStem(name.clone()))?,
            ),
            vec![parent],
            predicate,
        )
    } else {
        for search_dir in search_dirs {
            println!(
                "Searching for '{}' in '{}'",
                name.to_string_lossy(),
                search_dir.to_string_lossy()
            );

            let base_name = name
                .file_stem()
                .ok_or_else(|| Error::NoFileStem(name.clone()))?;
            let extension = name.extension();
            let ancestors = name
                .ancestors()
                .skip(1)
                .map(|p| p.to_path_buf())
                .collect::<Vec<_>>();

            let search_dir = absolute_path(PathBuf::from(&search_dir))?;

            // let results = WalkDir::new(search_dir)
            //     .contents_first(true)
            //     .into_iter()
            //     .flat_map(|entry| entry.map_err(Error::WalkDir))
            //     .filter(|entry| entry.path().is_dir())
            //     .filter_map(|entry| {
            //         entry
            //             .path()
            //             .file_stem()
            //             .map(|fs| fs.to_string_lossy().eq_ignore_ascii_case(base_name))
            //             .ok_or_else(|| Error::NoFileStem(entry.path().to_owned()))
            //             .and_then(|stems_eq| {
            //                 // TODO TODO TODO
            //                 extension
            //                     .map(|ext| ext.to_str().ok_or_else(|| Error::NonUtf8Path(ext.into())).map(|ext| ))
            //                     .transpose()
            //             });
            //         None
            //     });
            struct SearchResult {
                path: PathBuf,
                score: usize,
            }
            let mut results = vec![];

            for entry in WalkDir::new(search_dir).contents_first(true) {
                let entry = entry?;

                if entry.path().is_dir() {
                    continue;
                }

                if !predicate(entry.path()) {
                    continue;
                }

                let entry_extension = entry
                    .path()
                    .extension()
                    .map(|e| {
                        e.to_str()
                            .ok_or_else(|| Error::NonUtf8Path(entry.path().to_owned()))
                    })
                    .transpose()?
                    .unwrap_or("");

                let mut score = 0;
                let stem = entry
                    .path()
                    .file_stem()
                    .ok_or_else(|| Error::NoFileStem(entry.path().into()))?;
                let stems_eq = stem
                    .to_string_lossy()
                    .eq_ignore_ascii_case(base_name.to_string_lossy().as_ref());
                let stems_case_eq = stem.to_string_lossy() == base_name.to_string_lossy();
                let extensions_match = extension
                    .map(|ext| ext.to_string_lossy().eq_ignore_ascii_case(entry_extension))
                    .unwrap_or(true);
                let ancestors_eq = ancestors
                    .iter()
                    .zip(entry.path().ancestors().skip(1))
                    .all_equal();
                if stems_eq {
                    // doom2
                    score += 2;
                }
                if stems_case_eq {
                    // DOOM2
                    score += 5;
                }
                if extensions_match {
                    // Example.wad
                    score += 1;
                    if stems_eq {
                        // doom2.wad
                        score += 10;
                    }
                    if stems_case_eq {
                        score += 5;
                    }
                }
                if stems_eq && ancestors_eq {
                    // iwad/doom2
                    score += 20;
                }
                if score > 1 {
                    results.push(SearchResult {
                        path: entry.path().into(),
                        score,
                    });
                }
            }

            if !results.is_empty() {
                let results = results
                    .into_iter()
                    .sorted_by_key(|r| r.score)
                    .map(|r| r.path)
                    .rev()
                    .collect::<Vec<_>>();
                println!(
                    "Results: [{}]",
                    results
                        .iter()
                        .map(|r| r.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                return Ok(results);
            }
        }
        Err(Error::FileNotFound(name))
    }
}

fn select_between<P: AsRef<Path>>(
    search: impl AsRef<str>,
    options: impl AsRef<[P]>,
) -> Result<Vec<PathBuf>, Error> {
    MultiSelect::new()
        .with_prompt(format!("Multiple files were found for the search term {}. Please select one or more of the following:", search.as_ref()))
        .items(
            &options
                .as_ref()
                .iter()
                .map(|opt| opt.as_ref().to_string_lossy())
                .collect::<Vec<_>>(),
        )
        .interact()
        .map(|indices| indices.iter().map(|i| options.as_ref()[*i].as_ref().to_owned()).collect())
        .map_err(|e| e.into())
}

fn run_doom<'l>(mut cmdline: impl Iterator<Item = &'l str>) -> Result<(), Error> {
    let binary = PathBuf::from(cmdline.next().unwrap());
    if !binary.exists() {
        return Err(Error::FileNotFound(binary));
    }
    let binary_dir = {
        let mut d = binary.clone();
        d.pop();
        d
    };
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

#[derive(Serialize, Deserialize)]
struct Autoloads {
    universal: Vec<String>,
    sourceport: HashMap<String, Vec<String>>,
    iwad: HashMap<String, Vec<String>>,
}

fn autoload(pwads: &mut Pwads, engine: impl AsRef<Path>, iwad: &str) -> Result<(), Error> {
    let autoload_path = doom_dir()?.join("autoloads.json");
    let autoloads_file = File::open(&autoload_path).or_else(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            write!(
                File::create(&autoload_path).map_err(|e| {
                    Error::CreatingAutoloadsFile(e)
                })?,
                r#"
{{
    "_comment": "Place in 'universal' those PWADs that you always want to load.",
    "universal": [],
    "iwad": {{
        "_comment": ["Place in here those PWADs that only load under a specific IWAD. The key should be the IWAD, and the value the names of the PWADs."],
        "_example": ["foo.wad", "bar.pk3", "baz.zip"]
    }},
    "sourceport": {{
        "_comment": ["Place in here those PWADs that only load under a specific sourceport. The key should be the sourceport, and the value should be the PWADs."],
        "_example": ["foo.wad", "bar.pk3", "baz.zip"]
    }}
}}
            "#
            ).map_err(Error::Io)?;
            File::open(autoload_path).map_err(Error::OpeningFile)
        } else {
            Err(e.into())
        }
    })?;
    let reader = BufReader::new(autoloads_file);
    let autoloads: Autoloads = serde_json::from_reader(reader).unwrap();

    let universal_pwads = search_files(&autoloads.universal, FileType::Pwad)?;
    pwads.add_wads(universal_pwads);

    autoloads
        .sourceport
        .get(
            engine
                .as_ref()
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .as_ref(),
        )
        .map(|engine_specific_pwads| {
            pwads.add_wads(search_files(engine_specific_pwads, FileType::Pwad)?);
            Result::<(), Error>::Ok(())
        })
        .unwrap_or(Ok(()))?;
    if let Some(iwad_specific_pwads) = autoloads.iwad.get(iwad) {
        pwads.add_wads(search_files(iwad_specific_pwads, FileType::Pwad)?);
    }
    Ok(())
}

static CANCELLABLE: AtomicBool = AtomicBool::new(false);
static PAUSED: AtomicBool = AtomicBool::new(false);

fn run() -> Result<(), Error> {
    let app = App::new("Command-line Doom launcher")
            .version("0.1.0")
            .before_help("This Doom launcher allows shortcuts to the many long-winded options that Doom engines accept.")
            .setting(AppSettings::TrailingVarArg)
            .arg(Arg::with_name("3p").long("3p").help("Add the 3P Sound Pack"))
            .arg(Arg::with_name("compatibility-level").short("c").long("compatibility-level").help("Set the compatibility level to LEVEL").value_name("LEVEL"))
            .arg(Arg::with_name("debug").short("G").long("debug").help("Run Doom under a debugger"))
            .arg(Arg::with_name("engine").short("e").long("engine").help("Play the game with ENGINE instead of DSDA Doom").value_name("ENGINE"))
            .arg(Arg::with_name("extra-pwads").short("x").long("extra-pwads").help("Add PWADS to the game, silently").long_help("Silently means that when rendering a demo (with --render), the program will not add these PWADs to the folder name.").value_name("WAD").multiple(true))
            .arg(Arg::with_name("fast").short("f").long("fast").help("Enable fast monsters"))
            .arg(Arg::with_name("geometry").short("g").long("geometry").help("Set the screen resolution to WxH").long_help("Set the screen resolution to WxH; only supported on Boom-derived sourceports.").value_name("GEOM"))
            .arg(Arg::with_name("iwad").short("i").long("iwad").help("Set the game's IWAD").value_name("WAD"))
            .arg(Arg::with_name("no-monsters").long("no-monsters").help("Play the game with no monsters"))
            .arg(Arg::with_name("pistol-start").long("pistol-start").help("Play each level from a pistol start").long_help("Play each level from a pistol start. Currently only works with Crispy Doom and PrBoom+."))
            .arg(Arg::with_name("play-demo").short("d").long("play-demo").help("Play back DEMO").value_name("DEMO"))
            .arg(Arg::with_name("pwads").short("p").long("pwads").help("Add PWADS to the game").multiple(true).value_name("WAD"))
            .arg(Arg::with_name("record").short("r").long("record").help("Record a demo to DEMO").value_name("DEMO").long_help("Record a demo to DEMO, relative to ~/doom/demo."))
            .arg(Arg::with_name("record-from-to").long("record-from-to").number_of_values(2).help("Play back FROM, allowing you to rewrite its ending to TO").long_help("Play FROM. You are allowed to press the join key at any time to begin recording your inputs from the current frame. Whenever you quit the game, the final result will be written to TO.").value_names(&["FROM", "TO"]))
            .arg(Arg::with_name("render").short("R").long("render").help("Render a demo as a video").long_help("The video will be placed in /extra/Videos/{iwad}/{pwads}/{demoname}.").value_name("DEMO"))
            .arg(Arg::with_name("respawn").long("respawn").help("Enable respawning monsters"))
            .arg(Arg::with_name("short-tics").long("short-tics").help("Play the game with short tics instead of long tics"))
            .arg(Arg::with_name("skill").short("s").long("skill").help("Set the game's skill level by a number").value_name("SKILL"))
            .arg(Arg::with_name("vanilla-weapons").long("vanilla-weapons").help("Load the game with smooth weapon animations"))
            .arg(Arg::with_name("video-mode").short("v").long("video-mode").help("Set the video mode of the game (software, hardware)").long_help("Only supported on Boom-derived sourceports.").value_name("MODE"))
            .arg(Arg::with_name("warp").short("w").long("warp").help("Start the game at a specific level number").value_name("LEVEL"))
            .arg(Arg::with_name("passthrough").multiple(true))
            ;

    let matches = app.get_matches();

    let engine_name = matches.value_of("engine").unwrap_or("prboom-pp");
    let known_engines = read_known_engines()?;
    let engine = &known_engines.get(engine_name).unwrap_or_else(|| {
        eprintln!("ERROR: Unknown sourceport '{}'", engine_name);
        exit(-1);
    });

    let chosen_iwad = matches.value_of("iwad").unwrap_or("doom2");
    let iwad_path = search_file(chosen_iwad, FileType::Iwad)?;
    if iwad_path.is_empty() {
        eprintln!("IWAD not found: '{}'", chosen_iwad);
        exit(-1);
    }
    let iwad_path = absolute_path(&iwad_path[0])?;
    let iwad = iwad_path.to_string_lossy().to_string();

    let iwad_base = iwad_path
        .file_name()
        .ok_or_else(|| Error::NoFileStem(iwad_path.clone()))
        .and_then(|f| f.to_str().ok_or_else(|| Error::NonUtf8Path(f.into())))?;
    let iwad_noext = iwad_path
        .file_stem()
        .ok_or_else(|| Error::NoFileStem(iwad_path.clone()))
        .and_then(|i| i.to_str().ok_or_else(|| Error::NonUtf8Path(i.into())))?
        .to_lowercase();

    let mut cmdline = CommandLine::new();
    if matches.is_present("debug") {
        cmdline.push_line(Line::from_word("/usr/bin/lldb", 0));
    }
    cmdline.push_line(Line::from_word(
        engine
            .binary
            .to_str()
            .ok_or_else(|| Error::NonUtf8Path(engine.binary.clone()))?,
        0,
    ));
    if matches.is_present("debug") {
        cmdline.push_line(Line::from_word("--", 0));
    }
    if !engine.required_args.is_empty() {
        cmdline.push_line(Line::from_words(&engine.required_args, 1));
    }
    cmdline.push_line(Line::from_words(&["-iwad", &iwad], 1));

    let mut pwads = Pwads::new();

    if engine.supports_widescreen_assets {
        if let Ok(assets) = search_file(
            format!("{}_widescreen_assets.wad", iwad_noext),
            FileType::Pwad,
        ) {
            pwads.add_wads(assets);
        } else {
            print!("Couldn't find widescreen assets for ");
            print!(
                "{}",
                match iwad_noext.as_str() {
                    "doom" => "Doom",
                    "doom2" => "Doom 2",
                    "tnt" => "TNT: Evilution",
                    "plutonia" => "The Plutonia Experiment",
                    _ => "<unknown IWAD>",
                }
            );
            println!(".");
        }
    }

    let (sprite_fix, deh_fix) = match iwad_noext.as_str() {
        "doom2" | "tnt" | "plutonia" => (
            search_file("d2spfx19.wad", FileType::Pwad)?,
            search_file("d2dehfix.deh", FileType::Pwad)?,
        ),
        "doom" => (
            search_file("d1spfx19.wad", FileType::Pwad)?,
            search_file("d1dehfix.deh", FileType::Pwad)?,
        ),
        _ => (vec![], vec![]),
    };
    pwads.add_wads(sprite_fix);
    pwads.add_dehs(deh_fix);

    autoload(&mut pwads, &engine.binary, &iwad_noext)?;

    let mut viddump_folder_name = vec![];

    if let Some(arg_pwads_raw) = matches.value_of("pwads") {
        let mut arg_pwads = vec![];
        for pwad in arg_pwads_raw.split(ARG_SEPARATOR) {
            let mut pwad_files = search_file_by(pwad, FileType::Pwad, |f| {
                f.extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| {
                        ["wad", "pk3", "pk7", "pke", "zip", "deh", "bex"]
                            .contains(&ext.to_lowercase().as_str())
                    })
                    .unwrap_or(false)
            })?;
            viddump_folder_name.extend(
                search_file(pwad, FileType::Pwad)?
                    .iter()
                    .map(|p| {
                        p.file_stem()
                            .ok_or_else(|| Error::NoFileStem(p.to_owned()))
                            .and_then(|p| p.to_str().ok_or_else(|| Error::NonUtf8Path(p.into())))
                            .map(|p| p.to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );
            arg_pwads.append(&mut pwad_files);
        }
        for pwad in arg_pwads {
            match pwad
                .extension()
                .ok_or_else(|| Error::NoFileExtension(pwad.clone()))
                .and_then(|ext| ext.to_str().ok_or_else(|| Error::NonUtf8Path(ext.into())))?
                .to_lowercase()
                .as_str()
            {
                "wad" | "pk3" | "zip" | "pk7" | "pke" => pwads.add_wad(pwad),
                "deh" | "bex" => pwads.add_deh(pwad),
                _ => unreachable!(),
            }
        }
    }

    if let Some(extra_pwads) = matches.value_of("extra-pwads") {
        for pwad in extra_pwads.split(':') {
            pwads.add_wads(search_file(pwad, FileType::Pwad)?);
        }
    }

    if matches.is_present("vanilla-weapons") {
        pwads.add_wads(search_file("vsmooth.wad", FileType::Pwad)?);
        pwads.add_dehs(search_file("vsmooth.deh", FileType::Pwad)?);
    }

    if matches.is_present("3p") {
        let sound_pack = search_file("3P Sound Pack.wad", FileType::Pwad)?;
        pwads.add_wad(&sound_pack[0]);
    }

    if !pwads.wads().is_empty() {
        cmdline.push_line(Line::from_word("-file", 1));
        pwads.wads().iter().try_for_each(|pwad| {
            pwad.to_str()
                .ok_or_else(|| Error::NonUtf8Path(pwad.clone()))
                .map(|pwad| cmdline.push_line(Line::from_word(pwad, 2)))
        })?;
    }

    if !pwads.dehs().is_empty() {
        cmdline.push_line(Line::from_word("-deh", 1));
        pwads.dehs().iter().try_for_each(|deh| {
            deh.to_str()
                .ok_or_else(|| Error::NonUtf8Path(deh.into()))
                .map(|deh| cmdline.push_line(Line::from_word(deh, 2)))
        })?;
    }

    let complevel = matches.value_of("compatibility-level").unwrap_or("9");
    cmdline.push_line(Line::from_words(
        &[String::from("-complevel"), complevel.to_string()],
        1,
    ));

    if matches.is_present("pistol-start") {
        cmdline.push_line(Line::from_word("-pistolstart", 1));
    }

    let vidmode = matches.value_of("video-mode").unwrap_or("GL");
    cmdline.push_line(Line::from_words(&["-vidmode", vidmode], 1));

    let geom = matches.value_of("geometry").unwrap_or("2560x1440F");
    cmdline.push_line(Line::from_words(&["-geom", geom], 1));

    let skill_param = if engine.kind == DoomEngineKind::ZDoom {
        &["+skill", "3"]
    } else {
        &["-skill", "4"]
    };

    if let Some(recording_demo) = matches.value_of("record") {
        let demo_path = PathBuf::from(recording_demo);
        let demo_path = if demo_path.is_absolute() {
            demo_path
        } else {
            demo_dir()?.join(demo_path)
        };
        cmdline.push_line(Line::from_word("-record", 1));
        cmdline.push_line(Line::from_word(demo_path.to_string_lossy(), 2));
        if !matches.is_present("short-tics") {
            cmdline.push_line(Line::from_word("-longtics", 1));
        }
    } else if matches.is_present("short-tics") {
        cmdline.push_line(Line::from_word("-shorttics", 1));
    }

    if let Some(from_to) = matches.values_of("record-from-to") {
        let from_to = from_to.collect::<Vec<_>>();
        cmdline.push_line(Line::from_word("-recordfromto", 1));
        cmdline.push_line(Line::from_words(&from_to[0..2], 2));
    }

    if let Some(playing_demo) = matches.value_of("play-demo") {
        let demo = select_between(playing_demo, search_file(playing_demo, FileType::Demo)?)?;
        if demo.is_empty() {
            eprintln!("No such demo: {}", playing_demo);
            exit(-1);
        }
        cmdline.push_line(Line::from_word("-playdemo", 1));
        cmdline.push_line(Line::from_word(
            demo[0]
                .to_str()
                .ok_or_else(|| Error::NonUtf8Path(demo[0].clone()))?,
            2,
        ));
    }

    if let Some(warp) = matches.value_of("warp") {
        cmdline.push_line(Line::from_words(
            &{
                let mut words = vec!["-warp"];
                words.extend(warp.split(ARG_SEPARATOR));
                words
            },
            1,
        ));
    }

    if let Some(skill) = matches.value_of("skill") {
        cmdline.push_line(Line::from_words(&[skill_param[0], skill], 1));
    } else if matches.is_present("warp") {
        cmdline.push_line(Line::from_words(skill_param, 1));
    }

    if matches.is_present("no-monsters") {
        cmdline.push_line(Line::from_word("-nomonsters", 1));
    }

    if matches.is_present("fast") {
        cmdline.push_line(Line::from_word("-fast", 1));
    }

    if matches.is_present("respawn") {
        cmdline.push_line(Line::from_word("-respawn", 1));
    }

    let dump_dir = DUMP_DIR
        .join("Videos")
        .join(iwad_base)
        .join(viddump_folder_name.join(","));

    let mut renderings = if let Some(rendering) = matches.value_of("render") {
        rendering
            .split(':')
            .flat_map(|demo| {
                let results = search_file(demo, FileType::Demo).unwrap_or_else(|e| {
                    eprintln!("Error: {}", e);
                    exit(-1);
                });
                if results.is_empty() {
                    eprintln!("Failed to find demo '{}'", demo);
                    exit(-1);
                }
                results
            })
            .map(|demo_name| {
                let video_name = if dump_dir.exists() {
                    Ok(())
                } else {
                    create_dir_all(&dump_dir).map_err(|e| e.into())
                }
                .and_then(|_| {
                    demo_name
                        .file_stem()
                        .ok_or_else(|| Error::NoFileStem(demo_name.clone()))
                })
                .map(|viddump_filename| {
                    dump_dir.join({
                        let mut viddump_filename = viddump_filename.to_os_string();
                        viddump_filename.push(".mp4");
                        viddump_filename
                    })
                });
                video_name.map(|video_name| Job {
                    name: demo_name.file_stem().unwrap().to_str().unwrap().to_string(),
                    video_name,
                    demo_name,
                })
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![]
    };

    if let Some(passthrough) = matches.values_of("passthrough") {
        for arg in passthrough {
            cmdline.push_line(Line::from_word(arg, 1));
        }
    }

    println!();
    if renderings.is_empty() {
        println!("Command line: \n'\n{}'", cmdline);
        print!("Press enter to launch Doom.");
        stdout().flush().unwrap();
        stdin().read_line(&mut String::new()).unwrap();
        run_doom(cmdline.iter_words())?;
    }
    let (job_sender, job_receiver) = channel::<Result<Job, Error>>();
    let (unpause_sender, unpause_receiver) = channel::<()>();
    ctrlc::set_handler(move || {
        if CANCELLABLE.load(Ordering::Relaxed) {
            PAUSED.store(true, Ordering::SeqCst);
            let mut extra_demos = String::new();
            print!("Enter demo names, separated by spaces: ");
            stdout().flush().unwrap();
            stdin().read_line(&mut extra_demos).unwrap();

            if extra_demos.split_whitespace().next().is_none() {
                println!("You didn't enter any demo names.");
                return;
            }
            let jobs_sending_result = extra_demos
                .split_whitespace()
                .map(|d| search_file(d, FileType::Demo))
                .collect::<Result<_, _>>()
                .and_then(|d: Vec<_>| {
                    d.into_iter().flatten().try_for_each(|demo_name| {
                        let name = demo_name
                            .file_stem()
                            .ok_or_else(|| Error::NoFileStem(demo_name.clone()))
                            .map(|name| name.to_owned());
                        name.and_then(|name| {
                            let video_name = dump_dir.join({
                                let mut name = name.clone();
                                name.push(".mp4");
                                name
                            });
                            job_sender
                                .send(
                                    name.to_str()
                                        .ok_or_else(|| Error::NonUtf8Path(name.as_os_str().into()))
                                        .map(|name| Job {
                                            name: name.to_owned(),
                                            demo_name: demo_name.clone(),
                                            video_name,
                                        }),
                                )
                                .map_err(|e| Error::Send(e.to_string()))
                        })
                    })
                });

            if jobs_sending_result.is_err() {
                job_sender
                    .send(jobs_sending_result.map(|_| Job {
                        name: String::new(),
                        demo_name: PathBuf::new(),
                        video_name: PathBuf::new(),
                    }))
                    .unwrap();
            }

            PAUSED.store(false, Ordering::SeqCst);
            unpause_sender.send(()).unwrap();
        } else {
            println!();
            println!("Received interrupt, exiting. Goodbye.");
            exit(0);
        }
    })
    .map_err(Error::SignalHandler)?;
    let mut i = 1;
    while !renderings.is_empty() {
        println!("====== RENDERING QUEUE ======");
        for job in &renderings {
            println!(
                "{}  ==>  {}",
                job.demo_name
                    .to_str()
                    .ok_or_else(|| Error::NonUtf8Path(job.demo_name.clone()))?,
                job.name
            );
        }
        println!("==== END RENDERING QUEUE ====");

        let job = renderings.remove(0);
        let render_cmdline = {
            let mut rcmdline = cmdline.clone();
            rcmdline.push_line(Line::from_word("-timedemo", 1));
            rcmdline.push_line(Line::from_word(
                job.demo_name
                    .to_str()
                    .ok_or_else(|| Error::NonUtf8Path(job.demo_name.clone()))?,
                2,
            ));

            rcmdline.push_line(Line::from_word("-viddump", 1));
            rcmdline.push_line(Line::from_word(
                job.video_name
                    .to_str()
                    .ok_or_else(|| Error::NonUtf8Path(job.video_name.clone()))?,
                2,
            ));
            rcmdline
        };
        println!("Command line #{}: \n'\n{}'", i, render_cmdline);
        if i == 1 {
            let mut prompt = String::from("Press enter to begin ");
            if !renderings.is_empty() {
                prompt += "batch ";
            }
            print!("{}rendering.", prompt);
            stdout().flush().unwrap();
            stdin().read_line(&mut String::new()).unwrap();
        } else {
            CANCELLABLE.store(true, Ordering::SeqCst);
            println!("Continuing batch rendering in 10 seconds. Press <C-c> to add more demos to the queue.");
            sleep(Duration::from_secs(10));
            if PAUSED.load(Ordering::SeqCst) {
                unpause_receiver.recv().unwrap();
            }
            CANCELLABLE.store(false, Ordering::SeqCst);
            for job in job_receiver.try_iter() {
                renderings.push(job?);
            }
        }

        run_doom(render_cmdline.iter_words())?;

        i += 1;
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("ERROR: {}", e);
        exit(-1);
    }
}

#[derive(Debug)]
enum Error {
    BadJson {
        file: PathBuf,
        error: serde_json::Error,
    },
    CreatingAutoloadsFile(io::Error),
    FileNotFound(PathBuf),
    Fmt(std::fmt::Error),
    Homeless,
    Io(io::Error),
    NoFileExtension(PathBuf),
    NoFileStem(PathBuf),
    OpeningFile(io::Error),
    RunningDoom(io::Error),
    Send(String),
    SignalHandler(ctrlc::Error),
    NonUtf8Path(PathBuf),
    WalkDir(walkdir::Error),
}

impl From<io::Error> for Error {
    fn from(i: io::Error) -> Self {
        Self::Io(i)
    }
}

impl From<std::fmt::Error> for Error {
    fn from(f: std::fmt::Error) -> Self {
        Self::Fmt(f)
    }
}

impl From<walkdir::Error> for Error {
    fn from(w: walkdir::Error) -> Self {
        Self::WalkDir(w)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadJson { file, error } => {
                write!(
                    f,
                    "'{}' contains bad JSON: {}",
                    file.to_string_lossy(),
                    error
                )
            }
            Self::FileNotFound(path) => {
                write!(f, "file not found: '{}'", path.to_string_lossy())
            }
            Self::Fmt(err) => {
                write!(f, "formatter error: {}", err)
            }
            Self::Homeless => {
                write!(f, "you have no home :(")
            }
            Self::Io(err) => {
                write!(f, "I/O error: {}", err)
            }
            Self::NoFileExtension(path) => {
                write!(f, "no file extension in '{}'", path.to_string_lossy())
            }
            Self::NoFileStem(path) => {
                write!(f, "no file stem in '{}'", path.to_string_lossy())
            }
            Self::NonUtf8Path(path) => {
                write!(
                    f,
                    "The path '{}' is not valid UTF-8",
                    path.to_string_lossy()
                )
            }
            Self::RunningDoom(err) => {
                write!(f, "could not run Doom: {}", err)
            }
            Self::SignalHandler(err) => {
                write!(f, "could not create signal handler: {}", err)
            }
            Self::WalkDir(err) => {
                write!(f, "walking directory: {}", err)
            }
            Error::CreatingAutoloadsFile(err) => {
                write!(f, "creating autoloads.json in your Doom directory: {}", err)
            }
            Error::Send(err) => {
                write!(f, "attempting to send to job handler: {}", err)
            }
            Error::OpeningFile(err) => {
                write!(f, "attempting to open a file: {}", err)
            }
        }
    }
}

impl std::error::Error for Error {}
