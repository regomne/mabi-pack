use super::{FileInfo, HeadInfo, MabiError, HEADER_SIZE};
use byteorder::{LittleEndian, WriteBytesExt};
use libflate::zlib;
use mersenne_twister::MT19937;
use rand::{Rng, SeedableRng};
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, MAIN_SEPARATOR};
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

#[cfg(target_os = "windows")]
fn write_file_time(stm: &mut impl Write, root_dir: &str, rel_path: &str) -> Result<(), MabiError> {
    use std::mem::MaybeUninit;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use std::slice::from_raw_parts;
    use winapi::shared::minwindef::FILETIME;
    use winapi::um::fileapi::{CreateFileW, GetFileTime, OPEN_EXISTING};
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::winnt::{FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, GENERIC_READ};

    let fname = Path::new(root_dir).join(rel_path);
    let fname: Vec<u16> = fname
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let mut create_time: FILETIME = MaybeUninit::uninit().assume_init();
        let mut access_time: FILETIME = MaybeUninit::uninit().assume_init();
        let mut write_time: FILETIME = MaybeUninit::uninit().assume_init();
        let hf = CreateFileW(
            fname.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        );
        if hf == INVALID_HANDLE_VALUE {
            return Err(MabiError::IoFail(io::Error::new(
                io::ErrorKind::Other,
                "can't open file",
            )));
        }
        GetFileTime(hf, &mut create_time, &mut access_time, &mut write_time);
        let size = std::mem::size_of::<FILETIME>();
        stm.write_all(from_raw_parts(&create_time as *const _ as *const u8, size))?;
        stm.write_all(from_raw_parts(&create_time as *const _ as *const u8, size))?;
        stm.write_all(from_raw_parts(&access_time as *const _ as *const u8, size))?;
        stm.write_all(from_raw_parts(&write_time as *const _ as *const u8, size))?;
        stm.write_all(from_raw_parts(&write_time as *const _ as *const u8, size))?;
        CloseHandle(hf);
    }
    Ok(())
}

#[cfg(target_os = "unix")]
fn write_file_time(stm: &mut impl Write, root_dir: &str, rel_path: &str) -> Result<(), MabiError> {
    for _ in 0..40 {
        stm.write_u8(0)?;
    }
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

#[cfg(target_os = "windows")]
fn write_header_time(stm: &mut impl Write) -> Result<(), MabiError> {
    use std::mem::MaybeUninit;
    use std::slice::from_raw_parts;
    use winapi::shared::minwindef::FILETIME;
    use winapi::um::minwinbase::SYSTEMTIME;
    use winapi::um::sysinfoapi::GetSystemTime;
    use winapi::um::timezoneapi::SystemTimeToFileTime;
    unsafe {
        let mut sys_time: SYSTEMTIME = MaybeUninit::uninit().assume_init();
        GetSystemTime(&mut sys_time);
        let mut file_time: FILETIME = MaybeUninit::uninit().assume_init();
        SystemTimeToFileTime(&sys_time, &mut file_time);
        let size = std::mem::size_of::<FILETIME>();
        stm.write_all(from_raw_parts(&file_time as *const _ as *const u8, size))?;
        stm.write_all(from_raw_parts(&file_time as *const _ as *const u8, size))?;
    }
    Ok(())
}

#[cfg(target_os = "unix")]
fn write_header_time(_: &mut impl Write) -> Result<(), MabiError> {
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

/*
fn find_lcs_length(b1: &[u8], b2: &[u8]) -> usize {
    let min_len = if b1.len() < b2.len() {
        b1.len()
    } else {
        b2.len()
    };
    for i in 0..min_len {
        if b1[i] != b2[i] {
            return i;
        }
    }
    min_len
}

fn file_first_cmp_routine(s1: &String, s2: &String) -> Ordering {
    let b1 = s1.as_bytes();
    let b2 = s2.as_bytes();
    let eq_len = find_lcs_length(b1, b2);
    let b1 = &b1[eq_len..];
    let b2 = &b2[eq_len..];
    let b1_has_dir = b1.contains(&(MAIN_SEPARATOR as u32 as u8));
    let b2_has_dir = b2.contains(&(MAIN_SEPARATOR as u32 as u8));

    if b1_has_dir && !b2_has_dir {
        Ordering::Greater
    } else if !b1_has_dir && b2_has_dir {
        Ordering::Less
    } else {
        b1.partial_cmp(&b2).unwrap()
    }
}
*/

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
    //file_names.sort_by(file_first_cmp_routine);

    let index_size: u64 = file_names
        .iter()
        .map(|s| calc_str_size(s.as_bytes().len()).0 + 0x40)
        .sum::<usize>() as u64;

    let fs = File::create(output_fname)?;
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
