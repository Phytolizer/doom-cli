use crate::job::Job;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::RecvError;
use std::sync::mpsc::SendError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("'{file}' contains bad RON: {error}")]
    BadRon {
        file: PathBuf,
        error: ron::error::SpannedError,
    },
    #[error("creating autoloads file in your Doom directory: {0}")]
    CreatingAutoloadsFile(io::Error),
    #[error("file not found: '{0}'")]
    FileNotFound(String),
    #[error("formatter error: {0}")]
    Fmt(#[from] std::fmt::Error),
    #[error("Home directory not found (!)")]
    Homeless,
    #[error("I/O error: {0}")]
    Io(io::Error),
    #[error("Dialogue error: {0}")]
    Dialoguer(#[from] dialoguer::Error),
    #[error("no engines defined")]
    NoEngines,
    #[error("no file stem in '{0}'")]
    NoFileStem(String),
    #[error("attempting to open a file: {0}")]
    OpeningFile(io::Error),
    #[error("receiving from interrupt handler: {0}")]
    Recv(#[from] RecvError),
    #[error("could not run Doom: {0}")]
    RunningDoom(io::Error),
    #[error("sending to interrupt handler: {0}")]
    Send(Box<SendError<Result<Job, Error>>>),
    #[error("handling interrupt: {0}")]
    SignalHandler(ctrlc::Error),
    #[error("non-UTF-8 path: '{0}'")]
    NonUtf8Path(String),
    #[error("walking directory: {0}")]
    WalkDir(#[from] walkdir::Error),
}
