use byteorder::{LittleEndian, ReadBytesExt};
use clap::{App, Arg, SubCommand};
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use thiserror::Error as ThisError;

mod extract;
mod list;
mod pack;

pub const HEADER_SIZE: u64 = 0x220;

#[derive(ThisError, Debug)]
pub enum MabiError {
    #[error("io error: {0}")]
    IoFail(#[from] std::io::Error),

    #[error("format error")]
    WrongFormat,

    #[error("encoding error")]
    Encoding(#[from] std::string::FromUtf8Error),

    #[error("corrupted file")]
    CorruptedFile,

    #[error("unknown file path: {0}")]
    UnrecognizedPath(String),

    #[error("regular expression invalid: {0}")]
    InvalidRegexp(String),

    #[error("invalid version")]
    InvalidVersion,

    #[error("compress error: {0}")]
    CompressError(String),

    #[error("file time error")]
    TimeError,

    #[error("reading header fail: {0}")]
    ReadHeaderFail(String),

    #[error("reading index fail: {0}")]
    ReadIndexFail(String),

    #[error("error when extracting file:{0}, {1}")]
    ExtractFail(String, String),

    #[error("error in traversing the folder: {0}")]
    TraversingFail(String),

    #[error("error in processing:{0}, {1}")]
    PackingFail(String, String),

    #[error("Internal error")]
    InternalError,
}

#[derive(Debug)]
pub struct HeadInfo {
    file_ver: u32,
    file_cnt: u32,
    index_size: u32,
    content_size: u32,
}

#[derive(Debug)]
pub struct FileInfo {
    name: String,
    version: u32,
    off: u32,
    raw_size: u32,
    uncompr_size: u32,
}

fn read_c_str(mut stm: Vec<u8>) -> Result<String, MabiError> {
    let len = stm
        .iter()
        .position(|&c| c == 0)
        .ok_or(MabiError::WrongFormat)?;
    stm.resize(len, 0);
    String::from_utf8(stm).map_err(|e| MabiError::Encoding(e))
}

fn read_str(stm: &mut impl Read) -> Result<String, MabiError> {
    let str_size = match stm.read_u8()? as usize {
        n @ 0..=3 => (n + 1) * 16 - 1,
        4 => 6 * 16 - 1,
        5 => stm.read_u32::<LittleEndian>()? as usize,
        _ => return Err(MabiError::WrongFormat),
    };
    //@todo optimize this!
    let mut s: Vec<u8> = vec![0; str_size];
    stm.read_exact(&mut s)?;
    read_c_str(s)
}

pub fn read_header(stm: &mut BufReader<File>) -> Result<HeadInfo, MabiError> {
    let magic = stm.read_u32::<LittleEndian>()?;
    let pack_ver = stm.read_u32::<LittleEndian>()?;
    if magic != 0x4b434150 || pack_ver != 0x102 {
        return Err(MabiError::WrongFormat);
    }
    let file_ver = stm.read_u32::<LittleEndian>()?;
    let file_cnt = stm.read_u32::<LittleEndian>()?;
    stm.seek(SeekFrom::Current(0x1f0))?;

    if stm.read_u32::<LittleEndian>()? != file_cnt {
        return Err(MabiError::WrongFormat);
    }
    let index_size = stm.read_u32::<LittleEndian>()?;
    stm.seek(SeekFrom::Current(4))?;
    let content_size = stm.read_u32::<LittleEndian>()?;

    Ok(HeadInfo {
        file_ver,
        file_cnt,
        index_size,
        content_size,
    })
}

pub fn read_index(
    stm: &mut BufReader<File>,
    head_info: &HeadInfo,
) -> Result<Vec<FileInfo>, MabiError> {
    stm.seek(SeekFrom::Start(HEADER_SIZE))?;
    let mut index: Vec<u8> = vec![0; head_info.index_size as usize];
    stm.read_exact(&mut index)?;
    let mut index = Cursor::new(index);
    let mut files: Vec<FileInfo> = vec![];
    for _ in 0..head_info.file_cnt {
        let name = read_str(&mut index)?;
        let version = index.read_u32::<LittleEndian>()?;
        index.seek(SeekFrom::Current(4))?;
        let off = index.read_u32::<LittleEndian>()?;
        let raw_size = index.read_u32::<LittleEndian>()?;
        let uncompr_size = index.read_u32::<LittleEndian>()?;
        index.seek(SeekFrom::Current(0x2c))?;
        files.push(FileInfo {
            name,
            version,
            off,
            raw_size,
            uncompr_size,
        });
    }
    Ok(files)
}

fn main() {
    let args = App::new("Mabinogi pack utilities")
        .version("1.1.1")
        .author("regomne <fallingsunz@gmail.com>")
        .subcommand(
            SubCommand::with_name("pack")
                .about("Create a pack")
                .arg(
                    Arg::with_name("input")
                        .short("i")
                        .long("input")
                        .value_name("FOLDER")
                        .help("Set the input folder to pack")
                        .required(true),
                )
                .arg(
                    Arg::with_name("output")
                        .short("o")
                        .long("output")
                        .value_name("PACK_NAME")
                        .help("Set the output .pack file name")
                        .required(true),
                )
                .arg(
                    Arg::with_name("verkey")
                        .short("k")
                        .long("key-version")
                        .value_name("VER_KEY")
                        .help("Set the version (and will be used as a seed)")
                        .required(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("extract")
                .about("Extract a pack")
                .arg(
                    Arg::with_name("input")
                        .short("i")
                        .long("input")
                        .value_name("PACK_NAME")
                        .help("Set the input pack name to extract")
                        .required(true),
                )
                .arg(
                    Arg::with_name("output")
                        .short("o")
                        .long("output")
                        .value_name("FOLDER")
                        .help("Set the output folder")
                        .required(true),
                )
                .arg(
                    Arg::with_name("filter")
                        .short("f")
                        .long("filter")
                        .value_name("FILTER(S)")
                        .help(
                            "Set a filter when extracting, in regexp, multiple occurrences mean OR",
                        )
                        .number_of_values(1)
                        .multiple(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("list")
                .about("Output the file list of a pack")
                .arg(
                    Arg::with_name("input")
                        .short("i")
                        .long("input")
                        .value_name("PACK_NAME")
                        .help("Set the input pack name to list")
                        .required(true),
                )
                .arg(
                    Arg::with_name("output")
                        .short("o")
                        .long("output")
                        .value_name("LIST_FILE_NAME")
                        .help("Set the list file name, output to stdout if not set"),
                )
                .arg(
                    Arg::with_name("with-version")
                        .long("with-version")
                        .help("Print the version of every file"),
                ),
        )
        .get_matches();

    let ret = match if let Some(matches) = args.subcommand_matches("list") {
        list::run_list(
            matches.value_of("input").unwrap(),
            matches.value_of("output"),
            matches.is_present("with-version"),
        )
    } else if let Some(matches) = args.subcommand_matches("extract") {
        extract::run_extract(
            matches.value_of("input").unwrap(),
            matches.value_of("output").unwrap(),
            matches
                .values_of("filter")
                .map(|e| e.collect())
                .unwrap_or(vec![]),
        )
    } else if let Some(matches) = args.subcommand_matches("pack") {
        pack::run_pack(
            matches.value_of("input").unwrap(),
            matches.value_of("output").unwrap(),
            matches.value_of("verkey").unwrap(),
        )
    } else {
        println!("please select a subcommand (type --help to get details)");
        Ok(())
    } {
        Err(e) => {
            println!("Err: {}", e);
            1
        }
        _ => 0,
    };
    std::process::exit(ret);
}
