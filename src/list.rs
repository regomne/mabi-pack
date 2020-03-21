use super::{read_header, read_index, MabiError};
use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Write};

pub fn run_list(fname: &str, output: Option<&str>, has_version: bool) -> Result<(), MabiError> {
    let fs = File::open(fname)?;
    //let tra:Box<dyn Write> = Box::new(fs);
    let mut reader = BufReader::new(fs);
    let head_info = read_header(&mut reader)?;
    let file_entries = read_index(&mut reader, &head_info)?;

    let output_stream: Result<Box<dyn Write>, MabiError> =
        output.map_or(Ok(Box::new(io::stdout())), |path| {
            OpenOptions::new()
                .create(true)
                .write(true)
                .open(path)
                .map(|f| Box::new(f) as Box<dyn Write>)
                .map_err(|e| MabiError::IoFail(e))
        });

    let mut output_stream = output_stream?;

    if !has_version {
        file_entries.iter().for_each(|e| {
            writeln!(output_stream, "{}", e.name).unwrap();
        });
    } else {
        file_entries.iter().for_each(|e| {
            writeln!(output_stream, "{} {}", e.version, e.name).unwrap();
        });
    }
    Ok(())
}
