use std::io::Write;

use chrono::Utc;
use env_logger; //::{Builder, Env, fmt::{Color, Formatter}};
use log::LevelFilter;
use std::time::Duration;
use anyhow::{anyhow,Context};

#[allow(unused)]
pub fn print_type_of<T>(_: &T) {
    println!("{}", std::any::type_name::<T>())
}

pub fn init_log(level: LevelFilter) {
    let mut builder = env_logger::Builder::new();

    builder.format(|buf, record| {
        writeln!(buf, "{} [{:4}] [{}:{}] {:>5}: {} ", Utc::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                 std::thread::current().name().or(Some("unknown")).unwrap(),
                 record.file().unwrap(),
                 record.line().unwrap(),
                 record.level(),
                 record.args())
    });
    builder.filter_level(level);
    builder.init();


}

pub fn to_log_level(s: &str) -> anyhow::Result<LevelFilter, anyhow::Error> {
    match s {
        "off" | "o" => Ok(LevelFilter::Off),
        "error" | "e"  => Ok(LevelFilter::Error),
        "warn" | "w" => Ok(LevelFilter::Warn),
        "info" | "i" => Ok(LevelFilter::Info),
        "debug" | "d" => Ok(LevelFilter::Debug),
        "trace" | "t" => Ok(LevelFilter::Trace),
        _ => Err(anyhow::anyhow!("Error for log level: must be one of off, o, error, e, warn, w, info, i, debug, d, trace, t but got {}", &s))
    }
}

pub fn to_duration(s: &str) -> Result<Duration, anyhow::Error> {
    let mut num = String::new();
    let mut sum_secs = 0u64;
    for c in s.chars() {
        if c >= '0' && c <='9' {
            num.push(c);
        } else {
            let s = num.parse::<u64>().with_context(|| format!("cannot parse number {} inside duration {}", &num, &s))?;
            num.clear();
            match c {
                's' => sum_secs += s,
                'm' => sum_secs += s*60,
                'h' => sum_secs += s * 3600,
                'd' => sum_secs += s*3600*24,
                'w' => sum_secs += s*3600*24*7,
                _ => Err(anyhow!("Cannot interpret {} as a time unit inside duration {}", c, &s))?,
            }
        }
    }
    if num.len() > 0 {
        sum_secs += num.parse::<u64>().with_context(|| format!("cannot parse number {} inside duration {}", &num, &s))?;
    }
    Ok(Duration::from_secs(sum_secs))
}


const GREEK_SUFFIXES: &[&str] = &["B", "KB", "MB", "GB", "TB", "PB", "EB", "ZB", "YB"];
pub fn greek(v: f64) -> String {
    let mut number = v;
    let mut multi = 0;

    while number >= 1000.0 && multi < GREEK_SUFFIXES.len() - 1 {
        multi += 1;
        number /= 1024.0;
    }

    let mut s = format!("{}", number);
    s.truncate(4);
    if s.ends_with('.') {
        s.pop();
    }
    if s.len() < 4 { s.push(' '); }

    return format!("{:<5}{}", s, GREEK_SUFFIXES[multi]);
}
