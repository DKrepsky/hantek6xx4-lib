//! Run the driver without hardware on the scripted mock transport.
//!
//! `cargo run --example offline -p hantek6xx4-lib`

use hantek6xx4_lib::{Config, MockTransport, Scope};

fn main() -> hantek6xx4_lib::Result<()> {
    let mut mock = MockTransport::new();
    // the reset command is answered with the FPGA version word, here 0xd001
    mock.push_bulk_in([0x01u8, 0xd0].repeat(256));
    // identification and calibration reads: a blank info string and an invalid calibration table would be rejected,
    // so answer the calibration read with a table that carries the validity flag
    mock.on_control_in(|request, value, len| {
        let mut data = vec![0xffu8; len];
        match (request, value) {
            (0xB2, _) => data[0] = 1,
            (0xA2, 0x1600) => data[len - 2..].copy_from_slice(&0xFBCFu16.to_le_bytes()),
            (0xA2, 0x1B00) => data[len - 2..].copy_from_slice(&0xFACFu16.to_le_bytes()),
            _ => {}
        }
        data
    });

    let mut scope = Scope::new(mock)?;
    scope.configure(&Config::default())?;
    println!(
        "{} bulk packets were sent while configuring",
        scope.transport().bulk_writes().len()
    );
    Ok(())
}
