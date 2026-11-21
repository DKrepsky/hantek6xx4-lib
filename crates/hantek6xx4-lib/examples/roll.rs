//! Stream CH1 continuously (scan/roll style) for ten seconds at 500 ms/div.
//!
//! Run with a scope connected: `cargo run --example roll -p hantek6xx4-lib`

use hantek6xx4_lib::{Channel, Config, Error, Scope, TimeBase};
use std::time::{Duration, Instant};

fn main() -> hantek6xx4_lib::Result<()> {
    let mut scope = Scope::open()?;
    let mut cfg = Config::default();
    for (i, ch) in cfg.channels.iter_mut().enumerate() {
        ch.enabled = i == 0;
    }
    cfg.timebase = TimeBase::from_index(25)?; // 500 ms/div, one sample every 2 ms
    scope.configure(&cfg)?;
    scope.start_roll()?;

    let start = Instant::now();
    let mut total = 0usize;
    while start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(250));
        match scope.poll_roll() {
            Ok(frame) => {
                let new = frame.volts(Channel::Ch1).unwrap_or_default();
                total += new.len();
                if let Some(last) = new.last() {
                    println!("+{:3} samples, latest {last:.3} V", new.len());
                }
            }
            // polled too slowly: the capture RAM wrapped, the next poll continues with fresh data
            Err(Error::Overrun) => eprintln!("samples lost"),
            Err(e) => return Err(e),
        }
    }
    scope.stop()?;
    println!("{total} samples in 10 s");
    Ok(())
}
