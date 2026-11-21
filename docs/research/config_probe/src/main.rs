use rusb::{request_type, DeviceHandle, Direction, GlobalContext, Recipient, RequestType};
use std::time::Duration;

const T: Duration = Duration::from_millis(1500);
fn sl(ms: u64) { std::thread::sleep(Duration::from_millis(ms)); }

struct Dev { h: DeviceHandle<GlobalContext>, pkt: usize, cal: Vec<u16> }

impl Dev {
    fn ctrl_out(&self, req: u8, val: u16, data: &[u8]) -> rusb::Result<usize> {
        self.h.write_control(request_type(Direction::Out, RequestType::Vendor, Recipient::Device), req, val, 0, data, T)
    }
    fn ctrl_in(&self, req: u8, val: u16, len: usize) -> rusb::Result<Vec<u8>> {
        let mut b = vec![0u8; len];
        let n = self.h.read_control(request_type(Direction::In, RequestType::Vendor, Recipient::Device), req, val, 0, &mut b, T)?;
        b.truncate(n); Ok(b)
    }
    fn send(&mut self, data: &[u8]) -> rusb::Result<()> {
        let mut pre = [0u8; 10]; pre[..4].copy_from_slice(&[0x0f, 3, 3, 3]);
        self.ctrl_out(0xB3, 0, &pre)?;
        let r = self.ctrl_in(0xB2, 0, 10)?;
        self.pkt = if r.first().copied().unwrap_or(0) != 0 { 512 } else { 64 };
        self.h.write_bulk(0x02, data, T)?; Ok(())
    }
    fn recv(&self, len: usize) -> rusb::Result<Vec<u8>> {
        let mut b = vec![0u8; len];
        let n = self.h.read_bulk(0x86, &mut b, T)?; b.truncate(n); Ok(b)
    }
    fn spi(&mut self, d0: u8, d1: u8, reg: u8, cs: u8) -> rusb::Result<()> { self.send(&[8, 0, 0, d0, d1, reg, cs, 0]) }
    fn spi16(&mut self, v: u16, reg: u8, cs: u8) -> rusb::Result<()> { self.spi((v & 0xff) as u8, (v >> 8) as u8, reg, cs) }

    fn init(&mut self) -> rusb::Result<()> {
        self.send(&[0x0c, 0x00])?; let _ = self.recv(self.pkt)?;
        let c = self.ctrl_in(0xA2, 0x1600, 1154)?;
        self.cal = c.chunks(2).map(|x| u16::from_le_bytes([x[0], x[1]])).collect();
        self.spi(0x77, 0x47, 0x12, 4)?; sl(2);
        self.spi(0x03, 0x00, 0x33, 4)?; sl(2);
        self.spi(0x65, 0x00, 0x30, 2)?; sl(2);
        self.spi(0x28, 0xf1, 0x0f, 2)?; sl(2);
        self.spi(0x12, 0x38, 0x01, 2)?; sl(15);
        self.spi(0x3f, 0x00, 0x55, 4)?; // ADCCHModGain, PCB 1.06
        Ok(())
    }
}

fn gain(v: u8) -> u8 { match v { 0 => 0xd, 1 => 0xa, 2 => 7, 3 | 6 | 9 => 5, 4 | 7 | 10 => 2, _ => 0 } }
fn nch(mask: u8) -> u8 { (mask & 0xf).count_ones() as u8 }
// channel mode k = 1, 2, 4 (FUN_10001900)
fn kmode(mask: u8, t: u8) -> u8 {
    match nch(mask) { 0 => 0, 1 => if t < 7 { 1 } else if t == 7 { 2 } else { 4 }, 2 => if t > 7 { 4 } else { 2 }, _ => 4 }
}
fn interval_ns(t: u8, k: u8) -> f64 {
    if t <= 7 { return k as f64; }
    [4.0, 8.0, 20.0, 40.0, 80.0, 200.0, 400.0, 800.0, 2000.0, 4000.0, 8000.0, 20000.0][(t - 8) as usize]
}
fn divisor(t: u8) -> u32 {
    [1,1,1,1,1,1,1,1,1,1, 2,5,10,25,50,100,250,500,1000,2500,5000,10000,25000,50000,100000,250000,500000,1000000,2500000,5000000,10000000,25000000,50000000,100000000,250000000,500000000,500000000][t as usize]
}

// FUN_10002830: HMCAD1511 channel mode / gain
fn adc_cfg(d: &mut Dev, mask: u8, v: [u8; 4], t: u8) -> rusb::Result<()> {
    let g: Vec<u16> = v.iter().map(|&x| gain(x) as u16 & 0xf).collect();
    let quad = (g[3] << 12) | (g[2] << 8) | (g[1] << 4) | g[0];
    let en: Vec<usize> = (0..4).filter(|i| mask >> i & 1 == 1).collect();
    let (mut mode, mut m31, mut r3a, mut r3b, mut word) = (4u8, 0x204u16, 0x810u16, 0x204u16, quad);
    let n = en.len();
    if n == 1 && t < 7 {
        let u = g[en[0]]; word = u * 0x1111; mode = 1; m31 = 1;
        let r = match mask { 1 => 0x1010, 2 => 0x808, 4 => 0x404, _ => 0x202 }; r3a = r; r3b = r;
    } else if n == 1 && t == 7 {
        let u = g[en[0]]; word = u * 0x1111; mode = 2; m31 = 0x102;
        let (a, b) = match mask { 1 => (0x1010, 0x808), 2 => (0x404, 0x808), 4 => (0x404, 0x202), _ => (0x1010, 0x202) }; r3a = a; r3b = b;
    } else if n == 2 && t < 8 {
        mode = 2; m31 = 0x102;
        let lo = g[en[0]] * 0x11; let hi = g[en[1]] * 0x11; word = lo | (hi << 8);
        let (a, b) = match mask { 3 => (0x1010, 0x808), 5 => (0x1010, 0x404), 6 => (0x808, 0x404), 9 => (0x1010, 0x202), 10 => (0x808, 0x202), _ => (0x404, 0x202) }; r3a = a; r3b = b;
    }
    d.spi16(r3a, 0x3a, 4)?; sl(3);
    d.spi16(r3b, 0x3b, 4)?; sl(3);
    d.spi16(0, 0x0f, 4)?; sl(3);
    d.spi16(m31, 0x31, 4)?; sl(3);
    if mode == 4 { d.spi16(word, 0x2a, 4)?; }
    else {
        let (lo, hi) = ((word & 0xff) as u8, (word >> 8) as u8);
        d.spi(((hi ^ lo) & 0xf) ^ hi, lo & 0xf, 0x2b, 4)?;
    }
    Ok(())
}

fn relays(d: &mut Dev, v: [u8; 4], bw: [bool; 4], dc: [bool; 4]) -> rusb::Result<()> {
    let mut b = [0u8; 4];
    for i in 0..4 {
        let x = v[i];
        b[i] = (bw[i] as u8) << 7 | ((x > 8) as u8) << 6 | ((x <= 8) as u8) << 5 | ((x > 5) as u8) << 4 | ((x <= 5) as u8) << 3 | (dc[i] as u8) << 2 | 2;
    }
    d.send(&[8, 0, b[0], b[1], b[2], b[3], 1, 0])?; sl(4);
    d.send(&[8, 0, b[0] & 0x86, b[1] & 0x86, b[2] & 0x86, b[3] & 0x86, 1, 0])?; sl(50);
    Ok(())
}

fn trig_src(d: &mut Dev, t: u8, mask: u8, s: u8, peak: u8) -> rusb::Result<()> {
    let n = nch(mask);
    let (x, y): (u8, u8) = match n {
        1 => { let x = if t < 7 { 0 } else if t == 7 { 1 } else { (t != 8) as u8 + 2 }; (x, 3 - x) }
        2 => { let x = if t < 8 { 0 } else { (t != 8) as u8 + 1 }; (x, 2 - x) }
        _ => { let x = (t > 8) as u8; (x, 1 - x) }
    };
    let m = mask & 0xf;
    let uv6 = m >> s;
    let k = kmode(mask, t);
    let z: u8 = match k { 1 => 0, 2 => if n == 1 { if m == 1 || m == 4 { 0 } else { 1 } } else { (uv6 == 1) as u8 }, _ => s };
    d.send(&[0x12, 0, (peak & 1) << 6 | (m << 2) | x, 0, y & 3, ((!uv6) & 1) << 2 | z])
}

fn horiz(d: &mut Dev, t: u8, mask: u8, b: f64, hpos: f64) -> rusb::Result<()> {
    let k = kmode(mask, t); let f = interval_ns(t, k); let lat = 230.0;
    let pre = (hpos * b * 0.01).floor();
    let mut first = (lat + (pre + 100.0) * f) * 0.125; let mut second = (b - pre + 8.0) * f * 0.125;
    if f < 200.0 { first = first.ceil(); second = second.ceil(); } else { first = first.floor(); second = second.floor(); }
    let mut p = vec![0x10u8, 0];
    p.extend_from_slice(&(first as u64).to_le_bytes()[..6]); p.extend_from_slice(&(second as u64).to_le_bytes()[..6]);
    d.send(&p)
}

fn row(v: u8) -> usize { if v <= 5 { 5 } else if v <= 8 { 8 } else { 11 } }
// FUN: position (0..255) -> 16-bit word using the calibration table
fn pos_word(cal: &[u16], ch: usize, v: u8, k: u8, pos: u32) -> u16 {
    let m = match k { 4 => 4, 2 => 2, _ => 0 };
    let idx = m + 12 * (12 * ch + row(v));
    let (a, b) = (cal[idx] as i64, cal[idx + 1] as i64);
    let (lo, hi) = if matches!(v, 5 | 8 | 11) { (a, b) } else {
        let avg = (a + b + 1) >> 1; let mul = match v { 0 => 0.02, 1 => 0.05, 2 => 0.1, _ => 0.2 };
        let dd = (((b - avg) as f64) * mul + 0.5).floor() as i64; (avg - dd, avg + dd)
    };
    (((hi - lo) as f64 * pos as f64 / 255.0 + lo as f64) as i64) as u16
}
fn set_pos(d: &mut Dev, ch: usize, w: u16) -> rusb::Result<()> {
    let id = if ch == 0 { 0 } else { 1u8 << (ch - 1) };
    d.send(&[id, 0, (w & 0xff) as u8, (w >> 8) as u8])?; sl(10); Ok(())
}
fn trig_level(d: &mut Dev, npos: u32, sens: i32) -> rusb::Result<()> {
    let mid = ((npos as f64) * 200.0 / 256.0 + 28.5) as i32;
    let hi = (mid + sens).clamp(0, 0xe4) as u8; let lo = (mid - sens).max(0) as u8;
    let mut p = vec![0x07u8, 0];
    for _ in 0..4 { p.extend_from_slice(&[hi, hi, lo, lo]); }
    p.extend_from_slice(&[mid as u8; 8]); d.send(&p)
}
fn timebase(d: &mut Dev, t: u8) -> rusb::Result<()> {
    let x = divisor(t) - 1; let b = x.to_le_bytes();
    d.send(&[0x0f, 0, b[0], b[1], b[2], b[3]])
}

fn configure(d: &mut Dev, mask: u8, v: [u8; 4], t: u8, pos: [u32; 4]) -> rusb::Result<()> {
    let k = kmode(mask, t);
    d.spi(0x3f, 0x00, 0x55, 4)?;
    // SetSampleRate
    adc_cfg(d, mask, v, t)?; timebase(d, t)?; horiz(d, t, mask, 4096.0, 50.0)?;
    // SetCHAndTrigger
    relays(d, v, [std::env::var("BW").is_ok(); 4], [std::env::var("AC").is_err(); 4])?; adc_cfg(d, mask, v, t)?;
    let s = (0..4u8).find(|i| mask >> i & 1 == 1).unwrap();
    trig_src(d, t, mask, s, 0)?;
    for ch in 0..4 { let w = pos_word(&d.cal, ch, v[ch], k, pos[ch]); set_pos(d, ch, w)?; }
    trig_level(d, 128, 4)?;
    d.send(&[0x11, 0, 0, 0, 0, 0])?;
    Ok(())
}

fn capture(d: &mut Dev, mask: u8, t: u8) -> rusb::Result<Vec<Vec<u8>>> {
    d.send(&[0x03, 0, 0x01, 0])?;
    for _ in 0..40 { sl(50); d.send(&[0x06, 0])?; let r = d.recv(d.pkt)?; if r[0] & 2 != 0 { break; } }
    d.send(&[0x0d, 0])?; let _ = d.recv(d.pkt)?;
    d.send(&[0x0e, 0, 0, 0])?; d.send(&[0x05, 0, 0, 0x80])?;
    let mut data = Vec::new();
    while data.len() < 65536 { let c = d.recv(65536 - data.len())?; if c.is_empty() { break; } data.extend_from_slice(&c); }
    let _ = t;
    if std::env::var("RAW").is_ok() { std::fs::write(format!("raw_{mask:x}.bin"), &data).ok(); }
    let n = nch(mask) as usize; let stride = if n >= 3 { 4 } else { n };
    data.drain(..stride * 16); data.truncate(stride * 4000);
    let en: Vec<usize> = (0..4).filter(|i| mask >> i & 1 == 1).collect();
    let mut out = vec![Vec::new(); 4];
    for (j, &ch) in en.iter().enumerate() {
        out[ch] = data.iter().skip(j).step_by(stride).copied().collect();
    }
    Ok(out)
}
fn mean(v: &[u8]) -> f64 { v.iter().map(|&x| x as f64).sum::<f64>() / v.len().max(1) as f64 }

fn main() -> rusb::Result<()> {
    let which = std::env::args().nth(1).unwrap_or_else(|| "modes".into());
    let h = rusb::open_device_with_vid_pid(0x04b5, 0x6cde).expect("no device");
    h.claim_interface(0)?;
    let mut d = Dev { h, pkt: 512, cal: vec![] };
    d.init()?;
    println!("calibration words: {}, flag 0x{:04x}", d.cal.len(), d.cal[576]);
    if which == "modes" {
        // CH1 square wave through 1/2/4-channel modes at timebase 15: period and plateaus
        for (mask, name) in [(0x1u8, "1ch (CH1)"), (0x3, "2ch (CH1,CH2)"), (0xf, "4ch")] {
            configure(&mut d, mask, [8; 4], 15, [128; 4])?;
            let c = capture(&mut d, mask, 15)?;
            let s = &c[0]; let (mn, mx) = (*s.iter().min().unwrap(), *s.iter().max().unwrap());
            let mid = (mn as u16 + mx as u16) / 2;
            let e: Vec<usize> = (1..s.len()).filter(|&i| (s[i - 1] as u16) < mid && (s[i] as u16) >= mid).collect();
            let per: Vec<usize> = e.windows(2).map(|w| w[1] - w[0]).collect();
            let mut hist = std::collections::BTreeMap::new(); for &x in s.iter() { *hist.entry(x).or_insert(0usize) += 1; }
            let mut top: Vec<_> = hist.into_iter().collect(); top.sort_by_key(|x| std::cmp::Reverse(x.1));
            println!("{name}: samples/ch={} min={mn} max={mx} edges={} period(frames)={:?} top levels={:?}", s.len(), e.len(), &per[..per.len().min(4)], &top[..3]);
        }
    } else if which == "pos" {
        // baseline at pos 0/128/255 on unconnected channels for every vdiv, per mode
        for (mask, name) in [(0xcu8, "2ch CH3+CH4"), (0x4, "1ch CH3"), (0xf, "4ch")] {
            println!("== {name}");
            for v in 0..12u8 {
                let mut row_s = format!("v={v:2}:");
                for p in [0u32, 128, 255] {
                    configure(&mut d, mask, [v; 4], 15, [p; 4])?;
                    let c = capture(&mut d, mask, 15)?;
                    let chs: Vec<f64> = [2usize, 3].iter().filter(|&&ch| mask >> ch & 1 == 1).map(|&ch| mean(&c[ch])).collect();
                    row_s += &format!("  pos{p:3}: {}", chs.iter().map(|x| format!("{x:6.1}")).collect::<Vec<_>>().join("/"));
                }
                println!("{row_s}");
            }
        }
    }
    if which == "short" {
        for (mask, name) in [(0xfu8, "4ch"), (0x1, "1ch CH1")] {
            println!("== {name}: CH1 shorted, mean/sd (counts)");
            for v in 0..12u8 {
                let mut row_s = format!("v={v:2}:");
                for p in [0u32, 128, 255] {
                    configure(&mut d, mask, [v; 4], 15, [p; 4])?;
                    let c = capture(&mut d, mask, 15)?;
                    let s = &c[0]; let m = mean(s);
                    let sd = (s.iter().map(|&x| (x as f64 - m).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
                    row_s += &format!("  pos{p:3}: {m:6.1} sd {sd:4.2}");
                }
                println!("{row_s}");
            }
        }
    }
    if which == "bw" {
        for v in [0u8, 1, 3, 8] {
            configure(&mut d, 0xf, [v; 4], 15, [128; 4])?;
            let c = capture(&mut d, 0xf, 15)?;
            let s = &c[0]; let m = mean(s);
            let sd = (s.iter().map(|&x| (x as f64 - m).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
            println!("BW={} AC={} v={v}: mean {m:.1} sd {sd:.2} min {} max {}", std::env::var("BW").is_ok(), std::env::var("AC").is_ok(), s.iter().min().unwrap(), s.iter().max().unwrap());
        }
    }
    if which == "fast" {
        for (mask, t) in [(0x4u8, 3u8), (0x1, 3), (0x4, 6), (0x4, 7), (0x4, 8), (0xc, 3), (0xc, 7), (0xc, 8), (0xf, 3), (0xf, 7), (0xf, 8)] {
            configure(&mut d, mask, [8; 4], t, [128; 4])?;
            let c = capture(&mut d, mask, t)?;
            let mut msg = format!("mask {mask:#x} t={t:2} k={} f={}ns:", kmode(mask, t), interval_ns(t, kmode(mask, t)));
            for ch in 0..4 { if mask >> ch & 1 == 1 {
                let s = &c[ch]; let m = mean(s); let var = s.iter().map(|&x| (x as f64 - m).powi(2)).sum::<f64>() / s.len() as f64;
                msg += &format!("  CH{} n={} mean={:.1} sd={:.2} min={} max={}", ch + 1, s.len(), m, var.sqrt(), s.iter().min().unwrap(), s.iter().max().unwrap());
            } }
            println!("{msg}");
        }
    }
    d.h.release_interface(0)?;
    Ok(())
}
