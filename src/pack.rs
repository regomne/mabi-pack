use super::{FileInfo, HeadInfo, MabiError, HEADER_SIZE};
use byteorder::{LittleEndian, WriteBytesExt};
use libflate::zlib;
use mersenne_twister::MT19937;
use rand::{Rng, SeedableRng};
use std::fs::{metadata, File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, MAIN_SEPARATOR};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

fn pack_file(root_dir: &str, rel_path: &str, key: u32) -> Result<(FileInfo, Vec<u8>), MabiError> {
    let mut stm = vec![];
    {
        let mut fs = File::open(Path::new(root_dir).join(rel_path))?;
        fs.read_to_end(&mut stm)?;
    }
    let mut encoder = zlib::Encoder::new(vec![]).unwrap();
    encoder
        .write_all(&stm)
        .map_err(|e| MabiError::CompressError(e.to_string()))?;
    let mut encoded_buff = encoder
        .finish()
        .into_result()
        .map_err(|e| MabiError::CompressError(e.to_string()))?;

    let mut rng: MT19937 = SeedableRng::from_seed((key << 7) ^ 0xA9C36DE1);
    for i in 0..encoded_buff.len() {
        encoded_buff[i] ^= rng.next_u32() as u8;
    }

    Ok((
        FileInfo {
            name: rel_path.to_string(),
            version: key,
            off: 0,
            raw_size: encoded_buff.len() as u32,
            uncompr_size: stm.len() as u32,
        },
        encoded_buff,
    ))
}

fn calc_str_size(l: usize) -> (usize, u8) {
    match l {
        0..=14 => (16, 0),
        15..=30 => (32, 1),
        31..=46 => (48, 2),
        47..=62 => (64, 3),
        63..=94 => (96, 4),
        x => ((x + 21) / 16 * 16, 5),
    }
}

fn get_rel_path(root_dir: &str, full_path: &str) -> Result<String, MabiError> {
    let full_path = Path::new(full_path);
    let rel_name = full_path
        .strip_prefix(root_dir)
        .map_err(|_| MabiError::InternalError)?;
    Ok(rel_name.to_string_lossy().into_owned())
}

fn write_str_block(stm: &mut impl Write, s: &str) -> Result<u64, MabiError> {
    let (all_len, lead_byte) = calc_str_size(s.as_bytes().len());
    stm.write_u8(lead_byte)?;
    let mut wrote_bytes = 1;
    if lead_byte == 5 {
        stm.write_u32::<LittleEndian>(all_len as u32 - 5)?;
        wrote_bytes += 4;
    }
    stm.write_all(s.replace(MAIN_SEPARATOR, "\\").as_bytes())?;
    wrote_bytes += s.as_bytes().len();
    for _ in 0..all_len - wrote_bytes {
        stm.write_u8(0)?;
    }

    Ok(all_len as u64)
}

fn time_to_filetime(t: SystemTime) -> Result<u64, MabiError> {
    let t = t
        .duration_since(UNIX_EPOCH)
        .map_err(|_| MabiError::TimeError)?
        .as_millis();
    Ok(((t * 10000) + 116444736000000000) as u64)
}

fn write_file_time(stm: &mut impl Write, root_dir: &str, rel_path: &str) -> Result<(), MabiError> {
    let meta = metadata(Path::new(root_dir).join(rel_path))?;
    // As creation time is not supported in WSL, replace it with modified time
    //let c_time = time_to_filetime(meta.created()?)?;
    let a_time = time_to_filetime(meta.accessed()?)?;
    let m_time = time_to_filetime(meta.modified()?)?;
    stm.write_u64::<LittleEndian>(m_time)?;
    stm.write_u64::<LittleEndian>(m_time)?;
    stm.write_u64::<LittleEndian>(a_time)?;
    stm.write_u64::<LittleEndian>(m_time)?;
    stm.write_u64::<LittleEndian>(m_time)?;
    Ok(())
}

fn write_file_entry(
    stm: &mut impl Write,
    ent: &FileInfo,
    root_dir: &str,
) -> Result<u64, MabiError> {
    let str_block_size = write_str_block(stm, &ent.name)?;
    stm.write_u32::<LittleEndian>(ent.version)?;
    stm.write_u32::<LittleEndian>(0)?;
    stm.write_u32::<LittleEndian>(ent.off)?;
    stm.write_u32::<LittleEndian>(ent.raw_size)?;
    stm.write_u32::<LittleEndian>(ent.uncompr_size)?;
    stm.write_u32::<LittleEndian>(1)?;
    write_file_time(stm, root_dir, &ent.name)?;
    Ok(str_block_size + 0x40)
}

fn write_header_time(stm: &mut impl Write) -> Result<(), MabiError> {
    let cur = time_to_filetime(SystemTime::now())?;
    stm.write_u64::<LittleEndian>(cur)?;
    stm.write_u64::<LittleEndian>(cur)?;
    Ok(())
}

fn write_header(stm: &mut impl Write, head_info: &HeadInfo) -> Result<(), MabiError> {
    stm.write_u32::<LittleEndian>(0x4b434150)?;
    stm.write_u32::<LittleEndian>(0x102)?;
    stm.write_u32::<LittleEndian>(head_info.file_ver)?;
    stm.write_u32::<LittleEndian>(head_info.file_cnt)?;
    write_header_time(stm)?;
    stm.write("data\\".as_bytes())?;
    stm.write(&[0; 0x1e0 - 5])?;
    stm.write_u32::<LittleEndian>(head_info.file_cnt)?;
    stm.write_u32::<LittleEndian>(head_info.index_size)?;
    stm.write_u32::<LittleEndian>(0)?;
    stm.write_u32::<LittleEndian>(head_info.content_size)?;
    stm.write_all(&[0; 16])?;
    Ok(())
}

pub fn run_pack(input_folder: &str, output_fname: &str, version: &str) -> Result<(), MabiError> {
    let version = version
        .parse::<u32>()
        .map_err(|_| MabiError::InvalidVersion)?;
    let file_names: Vec<String> = WalkDir::new(input_folder)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_type().is_dir())
        .map(|e| get_rel_path(input_folder, e.into_path().to_str().unwrap()))
        .collect::<Result<Vec<String>, MabiError>>()
        .map_err(|e| MabiError::TraversingFail(e.to_string()))?;

    let index_size: u64 = file_names
        .iter()
        .map(|s| calc_str_size(s.as_bytes().len()).0 + 0x40)
        .sum::<usize>() as u64;

    let fs = OpenOptions::new()
        .create(true)
        .write(true)
        .open(output_fname)?;
    let mut stm = BufWriter::new(fs);
    stm.write_all(&[0; HEADER_SIZE as usize])?;
    let content_start_off = HEADER_SIZE + index_size;

    let mut content_off = 0;
    let mut index_off = HEADER_SIZE;
    for name in &file_names {
        let (mut fi, packed_file) = pack_file(input_folder, &name, version)
            .map_err(|e| MabiError::PackingFail(name.clone(), e.to_string()))?;
        fi.off = content_off;
        stm.seek(SeekFrom::Start(index_off))?;
        index_off += write_file_entry(&mut stm, &fi, input_folder)?;
        stm.seek(SeekFrom::Start(content_start_off + content_off as u64))?;
        stm.write_all(&packed_file)?;
        content_off += fi.raw_size;
    }

    let file_size = stm.seek(SeekFrom::Current(0))?;
    stm.seek(SeekFrom::Start(0))?;
    write_header(
        &mut stm,
        &HeadInfo {
            file_ver: version,
            file_cnt: file_names.len() as u32,
            index_size: index_size as u32,
            content_size: (file_size - content_start_off) as u32,
        },
    )?;

    Ok(())
}
