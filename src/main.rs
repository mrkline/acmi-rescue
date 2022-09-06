use std::{
    fs::File,
    io::{self, prelude::*},
};

use anyhow::{bail, Context, Result};
use camino::Utf8PathBuf;
use clap::Parser;
use flate2::read::DeflateDecoder;
use log::*;
use simplelog::*;
use zip::ZipWriter;

#[derive(Debug, Parser)]
struct Args {
    /// Verbosity (-v, -vv, -vvv, etc.)
    #[clap(short, long, parse(from_occurrences))]
    verbose: u8,

    #[clap(short, long, arg_enum, default_value = "auto")]
    color: Color,

    partial_acmi: Utf8PathBuf,
}

#[derive(Debug, Copy, Clone, clap::ArgEnum)]
enum Color {
    Auto,
    Always,
    Never,
}

fn run() -> Result<()> {
    let args = Args::parse();
    init_logger(&args);

    let fh = File::open(&args.partial_acmi)?;
    let acmi = unsafe { memmap::Mmap::map(&fh)? };
    drop(fh);

    let mut acmi: &[u8] = acmi.as_ref();
    let header = LocalFileHeader::parse_and_consume(&mut acmi);
    debug!("{header:?}");

    let decompressor = DeflateDecoder::new(io::Cursor::new(acmi));

    let mut zipper = ZipWriter::new(File::create("rescued.zip.acmi")?);

    let zip_opts = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);


    zipper.start_file("acmi.txt", zip_opts)?;

    // Splitting by lines will cut off any incomplete last line with no newline
    // to end it.
    for line in io::BufReader::new(decompressor).lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                if e.kind() != io::ErrorKind::InvalidInput {
                    bail!(e);
                } else {
                    break;
                }
            }
        };
        writeln!(zipper, "{line}")?;
    }

    zipper.finish()?;

    Ok(())
}

/// Reads a little-endian u32 from the front of the provided slice, shrinking it.
fn read_u32(input: &mut &[u8]) -> u32 {
    let (int_bytes, rest) = input.split_at(std::mem::size_of::<u32>());
    *input = rest;
    u32::from_le_bytes(int_bytes.try_into().expect("less than four bytes for u32"))
}

/// Reads a little-endian u16 from the front of the provided slice, shrinking it.
fn read_u16(input: &mut &[u8]) -> u16 {
    let (int_bytes, rest) = input.split_at(std::mem::size_of::<u16>());
    *input = rest;
    u16::from_le_bytes(int_bytes.try_into().expect("less than two bytes for u16"))
}

/// Local file header magic number
const LOCAL_FILE_HEADER_MAGIC: [u8; 4] = [b'P', b'K', 3, 4];

/// Data from a local file header
///
/// Each files' actual contents is preceded by this header.
/// These headers alllow for "streaming" decompression without
/// the use of the central directory.
#[derive(Debug)]
pub struct LocalFileHeader<'a> {
    pub minimum_extract_version: u16,
    pub flags: u16,
    pub compression_method: u16,
    pub last_modified_time: u16,
    pub last_modified_date: u16,
    pub crc32: u32,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub path: &'a [u8],
    pub extra_field: &'a [u8],
}

impl<'a> LocalFileHeader<'a> {
    pub fn parse_and_consume(header: &mut &'a [u8]) -> Self {
        // 4.3.7  Local file header:
        //
        // local file header signature     4 bytes  (0x04034b50)
        // version needed to extract       2 bytes
        // general purpose bit flag        2 bytes
        // compression method              2 bytes
        // last mod file time              2 bytes
        // last mod file date              2 bytes
        // crc-32                          4 bytes
        // compressed size                 4 bytes
        // uncompressed size               4 bytes
        // file name length                2 bytes
        // extra field length              2 bytes
        //
        // file name (variable size)
        // extra field (variable size)
        assert_eq!(header[..4], LOCAL_FILE_HEADER_MAGIC);
        *header = &header[4..];
        let minimum_extract_version = read_u16(header);
        let flags = read_u16(header);
        let compression_method = read_u16(header);
        let last_modified_time = read_u16(header);
        let last_modified_date = read_u16(header);
        let crc32 = read_u32(header);
        let compressed_size = read_u32(header);
        let uncompressed_size = read_u32(header);
        let path_length = read_u16(header) as usize;
        let extra_field_length = read_u16(header) as usize;
        let (path, remaining) = header.split_at(path_length);
        let (extra_field, remaining) = remaining.split_at(extra_field_length);
        *header = remaining;

        Self {
            minimum_extract_version,
            flags,
            compression_method,
            last_modified_time,
            last_modified_date,
            crc32,
            compressed_size,
            uncompressed_size,
            path,
            extra_field,
        }
    }
}

fn main() {
    run().unwrap_or_else(|e| {
        log::error!("{:?}", e);
        std::process::exit(1);
    });
}

/// Set up simplelog to spit messages to stderr.
fn init_logger(args: &Args) {
    let mut builder = ConfigBuilder::new();
    builder.set_target_level(LevelFilter::Off);
    builder.set_thread_level(LevelFilter::Off);
    builder.set_time_level(LevelFilter::Off);

    let level = match args.verbose {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    if level == LevelFilter::Trace {
        builder.set_location_level(LevelFilter::Error);
    }
    builder.set_level_padding(LevelPadding::Left);

    let config = builder.build();

    let color = match args.color {
        Color::Always => ColorChoice::AlwaysAnsi,
        Color::Auto => {
            if atty::is(atty::Stream::Stderr) {
                ColorChoice::Auto
            } else {
                ColorChoice::Never
            }
        }
        Color::Never => ColorChoice::Never,
    };

    TermLogger::init(level, config.clone(), TerminalMode::Stderr, color)
        .or_else(|_| SimpleLogger::init(level, config))
        .context("Couldn't init logger")
        .unwrap()
}
