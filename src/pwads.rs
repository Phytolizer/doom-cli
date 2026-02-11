use std::path::Path;
use std::path::PathBuf;

use crate::error::Error;
use crate::search::search_file;
use crate::search::search_path_by;
use crate::FileType;

pub(crate) struct Pwads {
    wads: Vec<PathBuf>,
    dehs: Vec<PathBuf>,
}

impl Pwads {
    pub(crate) fn new() -> Self {
        Self {
            wads: vec![],
            dehs: vec![],
        }
    }

    pub(crate) fn add_wads(&mut self, mut wads: Vec<PathBuf>) {
        self.wads.append(&mut wads);
    }

    pub(crate) fn add_wad(&mut self, wad: impl AsRef<Path>) {
        self.wads.push(wad.as_ref().to_owned());
    }

    pub(crate) fn add_deh(&mut self, deh: PathBuf) {
        self.dehs.push(deh);
    }

    pub(crate) fn wads(&self) -> &[PathBuf] {
        &self.wads
    }

    pub(crate) fn dehs(&self) -> &[PathBuf] {
        &self.dehs
    }
}

pub(crate) fn parse_arg_pwads(
    arg_pwads_raw: &[PathBuf],
    viddump_folder_name: &mut Vec<String>,
    pwads: &mut Pwads,
) -> Result<(), Error> {
    let mut arg_pwads = vec![];
    for pwad in arg_pwads_raw {
        let mut pwad_files = search_path_by(pwad, FileType::Pwad, |f| {
            f.extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| {
                    ["wad", "pk3", "pk7", "pke", "zip", "deh", "bex"]
                        .contains(&ext.to_lowercase().as_str())
                })
                .unwrap_or(true)
        })?;
        viddump_folder_name.extend(
            search_file(pwad, FileType::Pwad)?
                .iter()
                .map(|p| {
                    p.file_stem()
                        .ok_or_else(|| Error::NoFileStem(p.to_string_lossy().into_owned()))
                        .and_then(|p| {
                            p.to_str()
                                .ok_or_else(|| Error::NonUtf8Path(p.to_string_lossy().into_owned()))
                        })
                        .map(|p| p.to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
        let i = if pwad_files.len() > 1 {
            dialoguer::Select::new()
                .items(pwad_files.iter().map(|p| p.to_string_lossy()))
                .with_prompt(
                    format!(
                        "Multiple results were found for {}. Select one.",
                        pwad.display()
                    )
                    .as_str(),
                )
                .interact()?
        } else {
            0
        };
        arg_pwads.push(pwad_files.remove(i));
    }
    for pwad in arg_pwads {
        match pwad
            .extension()
            .map(|ext| {
                ext.to_str()
                    .ok_or_else(|| Error::NonUtf8Path(ext.to_string_lossy().into_owned()))
            })
            .transpose()?
            .unwrap_or("")
            .to_lowercase()
            .as_str()
        {
            "wad" | "pk3" | "zip" | "pk7" | "pke" | "" => pwads.add_wad(pwad),
            "deh" | "bex" => pwads.add_deh(pwad),
            _ => unreachable!(),
        }
    }
    Ok(())
}
