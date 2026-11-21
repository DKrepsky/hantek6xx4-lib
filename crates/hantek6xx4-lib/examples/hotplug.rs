//! Open a scope whenever one is plugged in and stop using it when it is removed.
//!
//! Run, then plug and unplug the scope: `cargo run --example hotplug -p hantek6xx4-lib`

use hantek6xx4_lib::{DeviceEvent, DeviceMonitor, Scope};
use std::time::Duration;

fn main() -> hantek6xx4_lib::Result<()> {
    let (_monitor, events) = DeviceMonitor::channel()?;
    println!("waiting for a scope, press Ctrl-C to quit");

    let mut open: Option<(hantek6xx4_lib::DeviceInfo, Scope<_>)> = None;
    loop {
        // handle events, and check on the open scope between them
        match events.recv_timeout(Duration::from_secs(1)) {
            Ok(DeviceEvent::Connected(info)) if open.is_none() => match Scope::open_device(info) {
                Ok(scope) => {
                    println!("connected {info}: {}", scope.info().name);
                    open = Some((info, scope));
                }
                Err(e) => eprintln!("could not open {info}: {e}"),
            },
            Ok(DeviceEvent::Disconnected(info)) => {
                println!("disconnected {info}");
                if open.as_ref().map(|(i, _)| *i) == Some(info) {
                    open = None; // drop the Scope: its USB handle is dead
                }
            }
            Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        if let Some((info, scope)) = open.as_mut() {
            // an operation on a removed device fails with Error::Disconnected
            match scope.state() {
                Ok(state) => println!("{info}: state {:#06x}", state.0),
                Err(e) if e.is_disconnected() => {
                    println!("{info}: the scope was removed");
                    open = None;
                }
                Err(e) => eprintln!("{info}: {e}"),
            }
        }
    }
}
