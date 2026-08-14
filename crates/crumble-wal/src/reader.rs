use std::fs::File;
use std::io::{ErrorKind, Read, Seek};
use std::path::Path;

use crate::error::WalError;
use crate::record::WalRecord;

pub fn read_all(path: impl AsRef<Path>) -> Result<Vec<(u64, WalRecord)>, WalError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };

    let mut records = Vec::new();

    loop {
        let lsn = file.stream_position()?;

        let mut len_bytes = [0u8; 4];
        match file.read_exact(&mut len_bytes) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err.into()),
        }
        let len = u32::from_le_bytes(len_bytes) as usize;

        let mut payload = vec![0u8; len];
        match file.read_exact(&mut payload) {
            Ok(()) => {}
            Err(_) => break,
        }

        match bincode::serde::decode_from_slice::<WalRecord, _>(
            &payload,
            bincode::config::standard(),
        ) {
            Ok((record, _)) => records.push((lsn, record)),
            Err(_) => break,
        }
    }

    Ok(records)
}
