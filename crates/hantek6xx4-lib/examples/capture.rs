//! Capture one frame from CH1 and print a short summary in volts.
//!
//! Run with a scope connected: `cargo run --example capture -p hantek6xx4-lib`

use hantek6xx4_lib::{CaptureOptions, Channel, Config, Scope};

fn main() -> hantek6xx4_lib::Result<()> {
    let mut scope = Scope::open()?;
    println!("{:?}", scope.info());

    // CH1 only, 1 V/div, 200 us/div, edge trigger on the rising edge at ADC code 128
    let mut cfg = Config::default();
    for (i, ch) in cfg.channels.iter_mut().enumerate() {
        ch.enabled = i == 0;
    }
    cfg.trigger.level = 128;
    scope.configure(&cfg)?;

    let frame = scope.capture(CaptureOptions::default())?;
    let volts = frame.volts(Channel::Ch1).expect("CH1 is enabled");
    let min = volts.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = volts.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    println!(
        "{} samples every {} ns, triggered: {}, min {min:.3} V, max {max:.3} V",
        volts.len(),
        frame.sample_interval_ns,
        frame.triggered
    );
    Ok(())
}
