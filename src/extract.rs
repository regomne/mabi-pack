use super::{read_header, read_index, FileInfo, HeadInfo, MabiError, HEADER_SIZE};
use libflate::zlib;
use mersenne_twister::MT19937;
use rand::{Rng, SeedableRng};
use regex::Regex;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;

fn make_regex(strs: Vec<&str>) -> Result<Vec<Regex>, MabiError> {
    strs.into_iter()
        .map(|s| {
            Regex::new(&s)
                .map_err(|e| MabiError::InvalidRegexp(s.to_string() + ":" + &e.to_string()))
        })
        .collect()
}

fn write_file(root_dir: &str, rel_path: &str, content: Vec<u8>) -> Result<(), MabiError> {
    let fname = Path::new(root_dir).join(rel_path);
    let par = fname.parent().ok_or(MabiError::UnrecognizedPath(
        fname.to_string_lossy().into_owned(),
    ))?;
    std::fs::create_dir_all(par)?;
    let mut fs = File::create(fname)?;
    fs.write_all(&content)?;
    Ok(())
}

fn extract_file(
    stm: &mut BufReader<File>,
    head_info: &HeadInfo,
    file_info: &FileInfo,
    root_dir: &str,
) -> Result<(), MabiError> {
    stm.seek(SeekFrom::Start(
        HEADER_SIZE + head_info.index_size as u64 + file_info.off as u64,
    ))?;
    let mut buff = vec![0; file_info.raw_size as usize];
    stm.read_exact(&mut buff)?;

    let mut rng: MT19937 = SeedableRng::from_seed((file_info.version << 7) ^ 0xA9C36DE1);
    for i in 0..buff.len() {
        buff[i] ^= rng.next_u32() as u8;
    }

    let mut decoder = zlib::Decoder::new(Cursor::new(buff)).unwrap();
    let mut decoded_buff = vec![];
    decoder.read_to_end(&mut decoded_buff)?;
    if decoded_buff.len() != file_info.uncompr_size as usize {
        return Err(MabiError::CorruptedFile);
    }
    write_file(root_dir, &file_info.name, decoded_buff)?;
    Ok(())
}

pub fn run_extract(fname: &str, output_folder: &str, filters: Vec<&str>) -> Result<(), MabiError> {
    let fs = File::open(fname)?;
    //let tra:Box<dyn Write> = Box::new(fs);
    let mut reader = BufReader::new(fs);
    let head_info =
        read_header(&mut reader).map_err(|e| MabiError::ReadHeaderFail(e.to_string()))?;
    let file_entries =
        read_index(&mut reader, &head_info).map_err(|e| MabiError::ReadIndexFail(e.to_string()))?;

    let filters = make_regex(filters)?;

    for fi in file_entries {
        if filters.len() == 0 || filters.iter().any(|re| re.find(&fi.name).is_some()) {
            extract_file(&mut reader, &head_info, &fi, output_folder)
                .map_err(|e| MabiError::ExtractFail(fi.name, e.to_string()))?;
        }
    }
    Ok(())
}
