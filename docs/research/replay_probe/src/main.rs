use rusb::{request_type, DeviceHandle, Direction, GlobalContext, Recipient, RequestType};
use std::time::Duration;

const T: Duration = Duration::from_millis(1500);

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect::<Vec<_>>().join(" ")
}

struct Dev { h: DeviceHandle<GlobalContext>, pkt: usize }

impl Dev {
    fn ctrl_out(&self, req: u8, val: u16, data: &[u8]) -> rusb::Result<usize> {
        self.h.write_control(request_type(Direction::Out, RequestType::Vendor, Recipient::Device), req, val, 0, data, T)
    }
    fn ctrl_in(&self, req: u8, val: u16, len: usize) -> rusb::Result<Vec<u8>> {
        let mut b = vec![0u8; len];
        let n = self.h.read_control(request_type(Direction::In, RequestType::Vendor, Recipient::Device), req, val, 0, &mut b, T)?;
        b.truncate(n);
        Ok(b)
    }
    // mirrors the vendor DLL: control 0xB3 (mode 1) + 0xB2 read, then bulk OUT
    fn send(&mut self, data: &[u8]) -> rusb::Result<()> {
        let mut pre = [0u8; 10];
        pre[..4].copy_from_slice(&[0x0f, 3, 3, 3]);
        self.ctrl_out(0xB3, 0, &pre)?;
        let r = self.ctrl_in(0xB2, 0, 10)?;
        self.pkt = if r.first().copied().unwrap_or(0) != 0 { 512 } else { 64 };
        let n = self.h.write_bulk(0x02, data, T)?;
        assert_eq!(n, data.len());
        Ok(())
    }
    fn recv(&self, len: usize) -> rusb::Result<Vec<u8>> {
        let mut b = vec![0u8; len];
        let n = self.h.read_bulk(0x86, &mut b, T)?;
        b.truncate(n);
        Ok(b)
    }
    fn spi(&mut self, d0: u8, d1: u8, reg: u8, cs: u8) -> rusb::Result<()> {
        self.send(&[8, 0, 0, d0, d1, reg, cs, 0])
    }
}

fn main() -> rusb::Result<()> {
    let stage: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let h = rusb::open_device_with_vid_pid(0x04b5, 0x6cde).expect("device not found / no permission");
    h.claim_interface(0)?;
    let mut d = Dev { h, pkt: 512 };

    // --- stage 1: reset, FPGA version, hw version, state
    d.send(&[0x0c, 0x00])?;
    let r = d.recv(d.pkt)?;
    println!("0C 00 reset -> {} bytes: {}", r.len(), hex(&r[..r.len().min(16)]));
    if r.len() >= 2 { println!("   FPGA version = 0x{:04x}", u16::from_le_bytes([r[0], r[1]])); }

    d.send(&[0x09, 0x00])?;
    let r = d.recv(d.pkt)?;
    println!("09 00 hw version -> {} bytes: {}", r.len(), hex(&r[..r.len().min(16)]));

    d.send(&[0x06, 0x00])?;
    let r = d.recv(d.pkt)?;
    println!("06 00 state -> {} bytes: {}", r.len(), hex(&r[..r.len().min(16)]));

    if stage >= 2 {
        // control EA then ADC/PLL init as in dsoInitHard
        println!("sending EA");
        match d.ctrl_out(0xEA, 0, &[0u8; 10]) { Ok(n)=>println!("EA ok {n}"), Err(e)=>println!("EA err {e} (continuing)") }
        println!("spi1"); d.spi(0x77, 0x47, 0x12, 4)?; std::thread::sleep(Duration::from_millis(2));
        println!("spi2"); d.spi(0x03, 0x00, 0x33, 4)?;
        std::thread::sleep(Duration::from_millis(2));  println!("pll1"); d.spi(0x65, 0x00, 0x30, 2)?;
        std::thread::sleep(Duration::from_millis(2));  println!("pll2"); d.spi(0x28, 0xf1, 0x0f, 2)?;
        std::thread::sleep(Duration::from_millis(15)); println!("pll3"); d.spi(0x12, 0x38, 0x01, 2)?;
        println!("ADC/PLL init sent");
        d.spi(0x3f, 0x00, 0x55, 4)?; // ADCCHModGain, PCB 1.06
        d.send(&[0x06, 0x00])?;
        let r = d.recv(d.pkt)?;
        println!("06 00 state after init -> {}", hex(&r[..r.len().min(16)]));
    }

    if stage >= 3 {
        let sl = |ms| std::thread::sleep(Duration::from_millis(ms));
        // timebase idx 15 (200us/div): divisor 100 -> 99
        d.send(&[0x0f, 0x00, 99, 0, 0, 0])?;
        // ADC 4-channel mode (HMCAD1511, cs=4), gain word for vdiv idx 8 (1V) = 0
        d.spi(0x10, 0x08, 0x3a, 4)?; sl(3);
        d.spi(0x04, 0x02, 0x3b, 4)?; sl(3);
        d.spi(0x00, 0x00, 0x0f, 4)?; sl(3);
        d.spi(0x04, 0x02, 0x31, 4)?; sl(3);
        d.spi(0x00, 0x00, 0x2a, 4)?;
        // relays: 1V/div DC, no BW limit -> 0x36 each; then & 0x86
        d.send(&[8, 0, 0x36, 0x36, 0x36, 0x36, 1, 0])?; sl(4);
        d.send(&[8, 0, 0x06, 0x06, 0x06, 0x06, 1, 0])?; sl(50);
        // trigger source: 4ch, t>8, source CH1
        d.send(&[0x12, 0x00, 0x3d, 0x00, 0x00, 0x00])?;
        // channel positions (values from the working pyhantek capture)
        d.send(&[0x00, 0x00, 0x01, 0x62])?; sl(10);
        d.send(&[0x01, 0x00, 0x64, 0x70])?; sl(10);
        d.send(&[0x02, 0x00, 0x80, 0x69])?; sl(10);
        d.send(&[0x04, 0x00, 0xd9, 0x72])?; sl(10);
        // trigger level: mid=78, +-4
        let mut tl = vec![0x07u8, 0x00];
        for _ in 0..4 { tl.extend_from_slice(&[82, 82, 74, 74]); }
        tl.extend_from_slice(&[78u8; 8]);
        d.send(&tl)?;
        // trigger mode: edge, rising, DC
        d.send(&[0x11, 0x00, 0x00, 0x00, 0x00, 0x00])?;
        // horizontal trigger position (hard-coded in open-source projects)
        let mut hp = vec![0x10u8, 0x00];
        hp.extend_from_slice(&214828u32.to_le_bytes()); hp.extend_from_slice(&[0, 0]);
        hp.extend_from_slice(&205600u32.to_le_bytes()); hp.extend_from_slice(&[0, 0]);
        d.send(&hp)?;
        println!("configured; starting capture (auto)");
        d.send(&[0x03, 0x00, 0x01, 0x00])?;
        let mut ready = false;
        for i in 0..50 {
            sl(100);
            d.send(&[0x06, 0x00])?;
            let r = d.recv(d.pkt)?;
            let st = u16::from_le_bytes([r[0], r[1]]);
            if i % 5 == 0 { println!("state[{i}] = 0x{st:04x}"); }
            if st & 2 != 0 { ready = true; println!("data ready after ~{} ms, state 0x{st:04x}", (i+1)*100); break; }
        }
        d.send(&[0x0d, 0x00])?;
        let r = d.recv(d.pkt)?;
        println!("0D buffer status: {}", hex(&r[..12]));
        d.send(&[0x0e, 0x00, 0x00, 0x00])?;
        d.send(&[0x05, 0x00, 0x00, 0x80])?;
        let mut data = Vec::new();
        while data.len() < 65536 {
            let chunk = d.recv(65536 - data.len())?;
            if chunk.is_empty() { break; }
            data.extend_from_slice(&chunk);
        }
        println!("ready={ready}, read {} bytes", data.len());
        for ch in 0..4 {
            let v: Vec<u8> = data.iter().skip(ch).step_by(4).copied().collect();
            let min = v.iter().min().unwrap(); let max = v.iter().max().unwrap();
            let mean = v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;
            println!("CH{} min={} max={} mean={:.1} first16: {}", ch+1, min, max, mean, hex(&v[..16]));
        }
        std::fs::write("capture.bin", &data).ok();
    }

    if stage >= 4 {
        let sl = |ms| std::thread::sleep(Duration::from_millis(ms));
        println!("sweep CH4 position word -> mean level");
        let mut w: u32 = 0x2000;
        while w <= 0xA000 {
            d.send(&[0x04, 0x00, (w & 0xff) as u8, (w >> 8) as u8])?; sl(10);
            d.send(&[0x03, 0x00, 0x01, 0x00])?;
            for _ in 0..30 { sl(50); d.send(&[0x06, 0x00])?; let r = d.recv(d.pkt)?; if r[0] & 2 != 0 { break; } }
            d.send(&[0x0d, 0x00])?; let _ = d.recv(d.pkt)?;
            d.send(&[0x0e, 0x00, 0x00, 0x00])?;
            d.send(&[0x05, 0x00, 0x00, 0x80])?;
            let mut data = Vec::new();
            while data.len() < 65536 { let c = d.recv(65536 - data.len())?; if c.is_empty() { break; } data.extend_from_slice(&c); }
            let v: Vec<f64> = data.iter().skip(3).step_by(4).map(|&x| x as f64).collect();
            let mean = v.iter().sum::<f64>() / v.len() as f64;
            println!("word 0x{:04x} ({:5}) -> CH4 mean {:.2}", w, w, mean);
            w += 0x0800;
        }
    }

    if stage >= 5 {
        let sl = |ms| std::thread::sleep(Duration::from_millis(ms));
        println!("trigger level sweep (normal mode, rising edge, source CH1)");
        let mut npos: i32 = 0;
        while npos <= 255 {
            let mid = ((npos as f64) * 200.0 / 256.0 + 28.5) as i32;
            let hi = (mid + 4).min(0xe4) as u8; let lo = (mid - 4).max(0) as u8; let m = mid.min(255) as u8;
            let mut tl = vec![0x07u8, 0x00];
            for _ in 0..4 { tl.extend_from_slice(&[hi, hi, lo, lo]); }
            tl.extend_from_slice(&[m; 8]);
            d.send(&tl)?;
            d.send(&[0x03, 0x00, 0x00, 0x00])?; // start, normal (not auto)
            let mut st = 0u16; let mut ready = false;
            for _ in 0..8 { sl(60); d.send(&[0x06, 0x00])?; let r = d.recv(d.pkt)?; st = u16::from_le_bytes([r[0], r[1]]); if st & 2 != 0 { ready = true; break; } }
            d.send(&[0x0d, 0x00])?; let r = d.recv(d.pkt)?;
            println!("nPos {:3} mid {:3} -> ready={} state=0x{:04x} 0D: {}", npos, mid, ready, st, hex(&r[..10]));
            if !ready { d.send(&[0x03, 0x00, 0x04, 0x00])?; sl(20); }
            npos += 16;
        }
    }

    if stage >= 6 {
        let sl = |ms| std::thread::sleep(Duration::from_millis(ms));
        let mid: i32 = 128;
        let hi = (mid + 4) as u8; let lo = (mid - 4) as u8;
        let mut tl = vec![0x07u8, 0x00];
        for _ in 0..4 { tl.extend_from_slice(&[hi, hi, lo, lo]); }
        tl.extend_from_slice(&[mid as u8; 8]);
        d.send(&tl)?;
        for i in 0..6 {
            d.send(&[0x03, 0x00, 0x00, 0x00])?;
            let mut ready = false;
            for _ in 0..10 { sl(60); d.send(&[0x06, 0x00])?; let r = d.recv(d.pkt)?; if r[0] & 2 != 0 { ready = true; break; } }
            d.send(&[0x0d, 0x00])?; let st = d.recv(d.pkt)?;
            d.send(&[0x0e, 0x00, 0x00, 0x00])?;
            d.send(&[0x05, 0x00, 0x00, 0x80])?;
            let mut data = Vec::new();
            while data.len() < 65536 { let c = d.recv(65536 - data.len())?; if c.is_empty() { break; } data.extend_from_slice(&c); }
            println!("cap {i}: ready={ready} 0D: {}", hex(&st[..12]));
            std::fs::write(format!("cap{i}.bin"), &data).ok();
            std::fs::write(format!("cap{i}.0d"), &st[..12]).ok();
        }
    }

    if stage >= 7 {
        let sl = |ms| std::thread::sleep(Duration::from_millis(ms));
        let mid: i32 = 128;
        let (hi, lo) = ((mid + 4) as u8, (mid - 4) as u8);
        let mut tl = vec![0x07u8, 0x00];
        for _ in 0..4 { tl.extend_from_slice(&[hi, hi, lo, lo]); }
        tl.extend_from_slice(&[mid as u8; 8]);
        d.send(&tl)?;
        if let Some(hp) = std::env::var("HPOS").ok().and_then(|v| v.parse::<f64>().ok()) {
            let (b, f, lat) = (4096.0f64, 800.0f64, 230.0f64);
            let pre = (hp * b * 0.01).floor();
            let first = ((lat + (pre + 100.0) * f) * 0.125).floor() as u64;
            let second = (((b - pre + 8.0) * f) * 0.125).floor() as u64;
            let mut p = vec![0x10u8, 0x00];
            p.extend_from_slice(&first.to_le_bytes()[..6]);
            p.extend_from_slice(&second.to_le_bytes()[..6]);
            println!("HPOS {hp}% -> first={first} second={second}");
            d.send(&p)?;
        }
        println!("windowed readback: start = end - 16384, read 16384 bytes");
        for i in 0..6 {
            d.send(&[0x03, 0x00, 0x00, 0x00])?;
            let mut ready = false;
            for _ in 0..10 { sl(60); d.send(&[0x06, 0x00])?; let r = d.recv(d.pkt)?; if r[0] & 2 != 0 { ready = true; break; } }
            d.send(&[0x0d, 0x00])?; let st = d.recv(d.pkt)?;
            let a0 = u16::from_le_bytes([st[0], st[1]]) as u32;
            let a2 = u16::from_le_bytes([st[2], st[3]]) as u32;
            let start = (a2.wrapping_sub(16384)) & 0xffff & !7;
            d.send(&[0x0e, 0x00, (start & 0xff) as u8, (start >> 8) as u8])?;
            d.send(&[0x05, 0x00, 0x00, 0x20])?;
            let mut data = Vec::new();
            while data.len() < 16384 { let c = d.recv(16384 - data.len())?; if c.is_empty() { break; } data.extend_from_slice(&c); }
            println!("cap {i}: ready={ready} a0={a0} a2={a2} start={start} got {} bytes", data.len());
            std::fs::write(format!("w{i}.bin"), &data).ok();
        }
    }
    d.h.release_interface(0)?;
    Ok(())
}
