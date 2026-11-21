//! Whole-driver tests against the scripted mock transport (no hardware needed).

use hantek6xx4_lib::protocol::{CAL_FLAG, CAL_WORDS};
use hantek6xx4_lib::transport::Call;
use hantek6xx4_lib::{CaptureOptions, Channel, Config, MockTransport, Scope, Sweep};

fn mock() -> MockTransport {
    let mut m = MockTransport::new();
    m.on_control_in(|req, value, len| {
        let mut v = vec![0u8; len];
        match (req, value) {
            (0xB2, _) => v[0] = 1,
            (0xA2, 0x1580) => {
                v.fill(0xff);
                v[..3].copy_from_slice(b"DSO");
                v[6..10].copy_from_slice(b"6000");
                v[14..19].copy_from_slice(b"P1.06");
            }
            (0xA2, 0x1600) => {
                let mut words = vec![0u16; CAL_WORDS];
                words[CAL_WORDS - 1] = CAL_FLAG;
                // CH1, row 8, quad mode
                words[100] = 13832;
                words[101] = 44132;
                v = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            }
            _ => {}
        }
        v
    });
    // reset reply: FPGA version 0xd001
    m.push_bulk_in([0x01u8, 0xd0].repeat(256));
    m
}

#[test]
fn init_reads_identification_and_programs_adc() {
    let scope = Scope::new(mock()).unwrap();
    let info = scope.info();
    assert_eq!(info.name, "6000");
    assert_eq!(info.pcb_version, "P1.06");
    assert_eq!(info.fpga_version, 0xd001);

    let writes = scope.transport().bulk_writes();
    assert_eq!(writes[0], vec![0x0c, 0x00]);
    assert_eq!(writes[1], vec![8, 0, 0, 0x77, 0x47, 0x12, 4, 0]);
    assert_eq!(writes.len(), 1 + 5);
}

#[test]
fn every_bulk_write_is_preceded_by_the_control_preamble() {
    let scope = Scope::new(mock()).unwrap();
    let calls = &scope.transport().calls;
    for (i, c) in calls.iter().enumerate() {
        if matches!(c, Call::BulkOut(_)) {
            assert!(
                matches!(calls[i - 2], Call::ControlOut { request: 0xB3, .. }),
                "call {i}"
            );
            assert!(
                matches!(calls[i - 1], Call::ControlIn { request: 0xB2, .. }),
                "call {i}"
            );
        }
    }
}

#[test]
fn configure_emits_verified_packets() {
    let mut scope = Scope::new(mock()).unwrap();
    let before = scope.transport().bulk_writes().len();
    scope.configure(&Config::default()).unwrap();
    let w: Vec<Vec<u8>> = scope.transport().bulk_writes().split_off(before);
    let has = |p: &[u8]| w.iter().any(|x| x == p);
    assert!(has(&[0x0f, 0, 99, 0, 0, 0]), "timebase 200us");
    assert!(has(&[8, 0, 0x36, 0x36, 0x36, 0x36, 1, 0]), "relays");
    assert!(has(&[8, 0, 0x06, 0x06, 0x06, 0x06, 1, 0]), "relays second write");
    assert!(has(&[0x12, 0, 0x3d, 0, 0, 0]), "trigger source");
    assert!(has(&[0, 0, 0x71, 0x71]), "CH1 position word 29041");
    assert!(has(&[8, 0, 0, 0x3f, 0, 0x55, 4, 0]), "ADC gain register for PCB 1.06");
    assert!(has(&[0x11, 0, 0, 0, 0, 0]), "edge rising");
    let h = w.iter().find(|x| x[0] == 0x10).unwrap();
    assert_eq!(h.len(), 14);
}

#[test]
fn configure_rejects_invalid_config() {
    let mut scope = Scope::new(mock()).unwrap();
    let mut cfg = Config::default();
    cfg.trigger.level = 250;
    assert!(scope.configure(&cfg).is_err());
}

#[test]
fn triggered_capture_reads_aligned_window_and_deinterleaves() {
    let mut scope = Scope::new(mock()).unwrap();
    let mut cfg = Config::default();
    cfg.channels[1].enabled = false; // CH1, CH3, CH4 -> 4-byte frames
    scope.configure(&cfg).unwrap();

    // scripted device: state ready+triggered, end address 0x4ff0, then 16384 bytes of ramp data
    let m = scope.transport_mut_for_test();
    m.push_bulk_in({
        let mut s = vec![0u8; 512];
        s[0] = 0x03;
        s[1] = 0xc4;
        s
    });
    m.push_bulk_in({
        let mut s = vec![0u8; 512];
        s[2] = 0xf0;
        s[3] = 0x4f;
        s
    });
    let data: Vec<u8> = (0..16384u32).map(|i| (i % 4) as u8 * 10 + (i / 4 % 7) as u8).collect();
    m.push_bulk_in(data);

    let frame = scope.capture(CaptureOptions::default()).unwrap();
    assert!(frame.triggered);
    assert_eq!(frame.trigger_sample, Some(2048));
    assert_eq!(frame.sample_interval_ns, 800.0);
    assert!(frame.channel(Channel::Ch2).is_none());
    let ch3 = frame.channel(Channel::Ch3).unwrap();
    assert_eq!(ch3.len(), 4096);
    assert_eq!(ch3[0], 20); // index 2 of each 4-byte frame
    assert_eq!(frame.channel(Channel::Ch1).unwrap()[3], 3);

    // readback started at end - 16384 aligned to 8: 0x4ff0 - 0x4000 = 0x0ff0
    let w = scope.transport().bulk_writes();
    assert!(w.contains(&vec![0x0e, 0, 0xf0, 0x0f]));
    assert!(w.contains(&vec![0x05, 0, 0x00, 0x20]));
}

#[test]
fn normal_sweep_times_out_without_trigger() {
    let mut scope = Scope::new(mock()).unwrap();
    let mut cfg = Config::default();
    cfg.trigger.sweep = Sweep::Normal;
    scope.configure(&cfg).unwrap();
    // mock answers state reads with zeros -> never ready
    let r = scope.capture(CaptureOptions {
        timeout: std::time::Duration::from_millis(30),
    });
    assert!(matches!(r, Err(hantek6xx4_lib::Error::Timeout(_))));
}
