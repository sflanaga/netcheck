#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unreachable_code)]

mod util;
mod cli;

use std::path::PathBuf;
use structopt::StructOpt;
use std::time::{Instant, Duration};
use anyhow::{anyhow, Context};
use log::{debug, error, info, trace, warn};
use log::LevelFilter;
use std::net::{TcpListener, TcpStream, SocketAddr, IpAddr, Ipv4Addr};
use std::io::{Read, Write};
use std::sync::{Arc,Mutex,Condvar};
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use util::{to_log_level, to_duration, to_size_usize};
use std::str::FromStr;
use std::thread::spawn;
use core::mem;
use humantime::parse_duration;
use lazy_static::lazy_static;
use crate::cli::Cli;

lazy_static! {
    pub static ref STAT: AtomicU64 = AtomicU64::new(0);

    pub static ref STOP_TICKER: AtomicBool = AtomicBool::new(false);

    pub static ref COND_STOP: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));
}

type Result<T> = anyhow::Result<T, anyhow::Error>;

fn main() {
    if let Err(err) = run() {
        error!("Error: {:?}", &err);
        std::process::exit(1);
    }
}


fn run() -> Result<()> {
    let cli: Cli = Cli::from_args();
    if cli.buff_size % 8 != 0 {
        return Err(anyhow!("buff size must be a multiple 8"));
    }


    util::init_log(cli.log_level);
    if let Some(socket_addr) = cli.server {
        let mut socket_addr = if  socket_addr.is_some() {
            let soc_str = socket_addr.unwrap();
            util::str_to_socketaddr(&soc_str)?
        } else {
            SocketAddr::new(IpAddr::from(Ipv4Addr::new(0, 0, 0, 0)), cli.port)
        };
        socket_addr.set_port(cli.port);

        info!("server listening to {}", &socket_addr);
        let listener = TcpListener::bind(socket_addr).with_context(|| format!("not a valid IP address: {}", &socket_addr))?;
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
    } else if let Some(mut socker_addr) = cli.client {
        socker_addr.set_port(cli.port);
        info!("client connecting to {}", &socker_addr);
        let mut stream = TcpStream::connect_timeout(&socker_addr, cli.timeout)?;
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

    let mut buf = vec![0u8; 1];
    stream.read_exact(&mut buf)?;

    match buf[0] {
        b'U' => {
            info!("receiving - client sent upload 'U' command");
            match recv_bytes(stream, buff_size, validate) {
                Ok(()) => {},
                Err(e) => info!("client stopped, reason: {}", e),
            }
        },
        b'D' => {
            info!("sending - client sent download 'D' command");
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
        info!("requesting uploading");
        stream.write(&[b'U'])?;
        send_bytes(stream, buff_size)?;
    } else {
        info!("requesting downloading");
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