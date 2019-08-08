#[macro_use]
extern crate slog;
extern crate clap;
extern crate slog_async;
extern crate slog_term;
#[macro_use]
extern crate failure;

use clap::{App, Arg, SubCommand};
use slog::Drain;
use std::path::PathBuf;

mod mount;
mod xar;

fn setup_logger(level: slog::Level) -> slog::Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::CompactFormat::new(decorator).build().fuse();
    let drain = slog::LevelFilter::new(drain, level).fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    slog::Logger::root(drain, o!())
}

fn main() {
    let archive_arg = Arg::with_name("archive")
        .index(1)
        .required(true)
        .help("/path/to/file.xar, the archive to be mounted");

    let matches = App::new("XAR Fuse")
        .arg(
            Arg::with_name("verbose")
                .long("verbose")
                .short("v")
                .help("display detailed output"),
        )
        .subcommand(SubCommand::with_name("header").arg(&archive_arg))
        .subcommand(
            SubCommand::with_name("mount").arg(&archive_arg).arg(
                Arg::with_name("print_only")
                    .short("n")
                    .help("print the mountpoint but don't mount"),
            ),
        )
        .get_matches();

    let level = if matches.is_present("verbose") {
        slog::Level::Debug
    } else {
        slog::Level::Info
    };

    let root_log = setup_logger(level);
    match matches.subcommand() {
        ("header", Some(sub_m)) => {
            let archive = sub_m.value_of("archive").unwrap();
            let header = xar::Header::from_file(PathBuf::from(archive)).unwrap();
            info!(&root_log, ""; "header" => format!("{:?}", header));
        }
        ("mount", Some(sub_m)) => {
            let archive = sub_m.value_of("archive").unwrap();
            let header = xar::Header::from_file(PathBuf::from(archive)).unwrap();
            let mount = header.find_mount().unwrap();
            if sub_m.is_present("print_only") {
                println!("{}", mount.to_str().unwrap());
                return;
            }
        }
        _ => panic!("invalid subcommand"),
    }
}
