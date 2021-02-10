use std::sync::{Arc,Mutex,Condvar};
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use structopt::StructOpt;
use lazy_static::lazy_static;
use anyhow::{anyhow, Context};
use std::net::{TcpListener, TcpStream, SocketAddr, IpAddr};
use std::time::{Instant, Duration};
use log::LevelFilter;
use humantime::parse_duration;

use crate::util::{to_log_level, to_duration, to_size_usize};

type Result<T> = anyhow::Result<T, anyhow::Error>;

lazy_static! {
    pub static ref BUILD_INFO: String  = format!("ver: {}  rev: {}  date: {}", env!("CARGO_PKG_VERSION"), env!("VERGEN_SHA_SHORT"), env!("VERGEN_BUILD_DATE"));
}

use crate::util::str_to_socketaddr;

#[derive(StructOpt, Debug, Clone)]
#[structopt(
version = BUILD_INFO.as_str(), rename_all = "kebab-case",
global_settings(& [
structopt::clap::AppSettings::ColoredHelp,
structopt::clap::AppSettings::DeriveDisplayOrder
]),
)]
pub struct Cli {
    #[structopt(short, long, conflicts_with("client"))]
    /// server mode - note ip:port binding address is optional
    pub server: Option<Option<String>>,

    #[structopt(short, long, conflicts_with("server"), parse(try_from_str = str_to_socketaddr))]
    /// client ip:port of server end to connect too
    pub client: Option<SocketAddr>,

    #[structopt(short, long, default_value("5s"), parse(try_from_str = parse_duration))]
    /// client ip:port of server end to connect too
    pub timeout: Duration,

    #[structopt(short = "B", long, default_value("256k"), parse(try_from_str = to_size_usize))]
    /// log level
    pub buff_size: usize,

    #[structopt(short, long, default_value("5150"))]
    /// port default to 5150 but this overrides that
    pub port: u16,

    #[structopt(short, long)]
    /// client validate buf
    pub exambuf: bool,

    #[structopt(short, long, conflicts_with_all = &["server"])]
    /// client requests uploading - default is to download from server
    pub upload: bool,

    #[structopt(short = "L", long, parse(try_from_str = to_log_level), default_value("info"))]
    /// log level
    pub log_level: LevelFilter,

}

