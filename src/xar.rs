extern crate failure;
extern crate serde_aux;

use serde::Deserialize;
use serde_aux::prelude::deserialize_number_from_string;
use std::convert::TryInto;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::PathBuf;

const DEFAULT_HEADER_SIZE: usize = 4 * 1024;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "UPPERCASE")]
pub struct XarHeader {
    #[serde(deserialize_with = "deserialize_number_from_string")]
    pub offset: u64,
    pub version: String,
    pub xarexec_target: String,
    pub uuid: String,
    pub mount_root: Option<String>,
}

pub struct Xar {
    pub archive: PathBuf,
    pub logger: slog::Logger,
    pub header: XarHeader,
}

impl Xar {
    pub fn from_file(archive_path: PathBuf, logger: slog::Logger) -> Result<Xar, failure::Error> {
        let file = File::open(&archive_path)?;
        let mut reader = BufReader::with_capacity(DEFAULT_HEADER_SIZE, file);

        loop {
            let mut buf = String::new();
            let read = reader.read_line(&mut buf)?;
            match read {
                0 => return Err(format_err!("malformed header, no #xar_stop")),
                _n => {
                    if buf.starts_with("#xar_stop") {
                        let offset = reader.seek(SeekFrom::Current(0))?;
                        reader.seek(SeekFrom::Start(0))?;
                        let mut buffer = vec![0; offset.try_into().unwrap()];
                        let _read = reader.read(&mut buffer)?;
                        let header: XarHeader = toml::from_slice(&buffer)?;
                        return Ok(Xar {
                            archive: PathBuf::from(&archive_path),
                            logger: logger,
                            header: header,
                        });
                    }
                }
            }
        }
    }
}
