#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unreachable_code)]


mod util;

use std::path::PathBuf;
use structopt::StructOpt;
use std::time::{Instant, Duration};
use anyhow::{anyhow, Context};
use log::{debug, error, info, trace, warn};
use log::LevelFilter;
use lazy_static::lazy_static;
use std::net::{TcpListener, TcpStream, SocketAddr};
use std::io::{Read, Write};
use std::sync::{Arc,Mutex,Condvar};
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use util::{to_log_level, to_duration, to_size_usize};
use std::str::FromStr;
use std::thread::spawn;
use core::mem;
use humantime::parse_duration;

type Result<T> = anyhow::Result<T, anyhow::Error>;

lazy_static! {
    pub static ref BUILD_INFO: String  = format!("ver: {}  rev: {}  date: {}", env!("CARGO_PKG_VERSION"), env!("VERGEN_SHA_SHORT"), env!("VERGEN_BUILD_DATE"));

    pub static ref STAT: AtomicU64 = AtomicU64::new(0);

    pub static ref STOP_TICKER: AtomicBool = AtomicBool::new(false);

    pub static ref COND_STOP: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));
}

#[derive(StructOpt, Debug, Clone)]
#[structopt(
version = BUILD_INFO.as_str(), rename_all = "kebab-case",
global_settings(& [
structopt::clap::AppSettings::ColoredHelp,
structopt::clap::AppSettings::UnifiedHelpMessage
]),
)]
pub struct Cli {
    #[structopt(short, long, conflicts_with("client"))]
    /// server ip:port binding address
    pub server: Option<String>,

    #[structopt(short, long, conflicts_with("server"))]
    /// client ip:port of server end to connect too
    pub client: Option<String>,

    #[structopt(short, long, default_value("5s"), parse(try_from_str = parse_duration))]
    /// client ip:port of server end to connect too
    pub timeout: Duration,

    #[structopt(short = "B", long, default_value("256k"), parse(try_from_str = to_size_usize))]
    /// log level
    pub buff_size: usize,

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

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {:?}", &err);
        std::process::exit(1);
    }
}


fn run() -> Result<()> {
    let cli: Cli = Cli::from_args();
    if cli.buff_size % 8 != 0 {
        return Err(anyhow!("buff size must be a multiple 8"));
    }
    util::init_log(cli.log_level);
    if let Some(ip_str) = cli.server {
        println!("server listening to {}", &ip_str);
        let addr: SocketAddr = SocketAddr::from_str(&ip_str)?;
        let listener = TcpListener::bind(addr).with_context(|| format!("not a valid IP address: {}", &ip_str))?;
        // accept connections and process them serially
        for stream in listener.incoming() {
            let stream = stream?;
            stream.set_read_timeout(Some(cli.timeout))?;
            stream.set_write_timeout(Some(cli.timeout))?;
            let client_addr = stream.peer_addr()?;
            info!("Connection from: {:?}", &client_addr);
            spawn_ticker();
            match server(stream, cli.buff_size, cli.exambuf) {
                Err(e) => error!("Going back to listening, error {:?}", e),
                Ok(()) => info!("client {} done - going back to listening", &client_addr),
            }
            stop_ticker();
        }
    } else if let Some(ip_str) = cli.client {
        println!("client connecting to {}", &ip_str);
        let addr: SocketAddr = SocketAddr::from_str(&ip_str).with_context(|| format!("not a valid IP address: {}", &ip_str))?;
        let mut stream = TcpStream::connect_timeout(&addr, cli.timeout)?;
        stream.set_read_timeout(Some(cli.timeout))?;
        stream.set_write_timeout(Some(cli.timeout))?;
        spawn_ticker();
        client(stream, cli.buff_size, cli.exambuf, cli.upload)?;
    } else {
        return Err(anyhow!("Error - either server or client must be specified"))?;
    }


    Ok(())
}


fn server(mut stream: TcpStream, buff_size: usize, validate: bool) -> Result<()> {
    let mut buf = vec![0; buff_size];

    let cmd_size = stream.read(&mut buf)?;

    if cmd_size != 1 {
        return Err(anyhow!("expected command from clinet - U or D, but got message of size: {}", cmd_size));
    }

    match buf[0] {
        b'U' => {
            println!("receiving - client sent upload 'U' command");
            match recv_bytes(stream, buff_size, validate) {
                Ok(()) => {},
                Err(e) => println!("cliented stopped?  msg: {}", e),
            }
        },
        b'D' => {
            println!("sending - client sent download 'D' command");
            send_bytes(stream, buff_size)?;
        },
        b => {
            return Err(anyhow!("Cmd \"{}\" not understood", b));
        }

    }
    return Ok(());
}

fn client(mut stream: TcpStream, buff_size: usize, validate: bool, upload: bool) -> Result<()> {
    if upload {
        println!("requesting uploading");
        stream.write(&[b'U'])?;
        send_bytes(stream, buff_size)?;
    } else {
        println!("requesting downloading");
        stream.write(&[b'D'])?;
        recv_bytes(stream, buff_size, validate)?;
    }

    Ok(())
}



fn send_bytes(mut stream: TcpStream, buff_size: usize) -> Result<()> {
// fill buf with deadbeef
    let mut buf = vec![0; 64 * 1024];
    let r = [0xaa, 0xaa, 0xaa, 0xa];
    for (c, b) in buf.iter_mut().enumerate() {
        let i = c & 0x03;
        *b = 0xaa; // r[i];
    }
    loop {
        stream.write_all(&mut buf)?;
        STAT.fetch_add(buf.len() as u64, Ordering::Relaxed);
    }
    return Ok(());
}

fn recv_bytes(mut stream: TcpStream, buff_size: usize, validate: bool) -> Result<()> {
    let mut validate_buf = vec![0u8; buff_size];
    // fill buf with deadbeef
    //let r = [0xaa, 0xaa, 0xaa, 0xaa];
    for (c, b) in validate_buf.iter_mut().enumerate() {
        let i = c & 0x03;
        *b = 0xaa;
    }

    let mut buf = vec![0; 64 * 1024];
    loop {
        let size = stream.read_exact(&mut buf)?;
        STAT.fetch_add(buf.len() as u64, Ordering::Relaxed);
        if validate {

            // well this is the simplest unsafe code
            let length = (buf.len() / 8) as isize;
            let capacity = buf.capacity() / 8;
            let ptr = buf.as_mut_ptr() as *mut u64;

            unsafe {
                for count in 0isize..length {
                    //info!("ptr: {:x} cnt: {}", *ptr, length);
                    if *ptr.offset(count) != 0xaaaa_aaaa_aaaa_aaaa {
                        return Err(anyhow!("error in beef - got {:x} instead - there might be endian problem on this arch", *ptr));
                    }
                }
            };

        }

    }
}

fn stop_ticker() {
    let mut lock = COND_STOP.0.lock().unwrap();
    *lock = true;
    COND_STOP.1.notify_all();
}

fn spawn_ticker() {
    STAT.store(0, Ordering::Relaxed);
    {
        let mut lock = COND_STOP.0.lock().unwrap();
        *lock = false;
    }

    let h = spawn(move || {
        let mut last_sz = STAT.fetch_add(0, Ordering::Relaxed);
        loop {
            {
                let lock = COND_STOP.0.lock().unwrap();
                if !*lock {
                    let res = COND_STOP.1.wait_timeout(lock, Duration::from_secs(1)).unwrap();
                    if *res.0 {
                        debug!("stopping on check of condition during or interrupted sleep");
                        break;
                    }
                } else {
                    debug!("stopping on initial check of condition before sleep");
                    break;
                };
            }
            if STOP_TICKER.load(Ordering::Relaxed) == true {
                info!("tic stopped");
                break;
            }
            let thissize = STAT.fetch_add(0, Ordering::Relaxed);
            let rate = (thissize - last_sz) as f64;
            info!("tic: {} rate: {}", thissize, util::greek(rate));
            last_sz = thissize;
        }
    });
}